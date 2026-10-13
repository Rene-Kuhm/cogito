//! Code Fingerprint Generator
//!
//! Creates minimal structural snapshots of codebases with real git analysis.

use crate::types::*;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Time thresholds for zone frequency classification
const HOT_THRESHOLD_DAYS: i64 = 30;
const WARM_THRESHOLD_DAYS: i64 = 90;
const FROZEN_THRESHOLD_DAYS: i64 = 180;
const HOT_MIN_COMMITS: usize = 10;
const WARM_MIN_COMMITS: usize = 3;

/// Generate fingerprint for a project
pub async fn generate(project: &Project) -> Result<Fingerprint> {
    let root = &project.path;
    let entry_points = find_entry_points(root, &project.language)?;
    let mut data_flows = analyze_data_flows(root, &project.language)?;
    let zones = detect_zones_path(root)?;

    // Every project file, relative to the root: the layout is derived from it
    // and import targets are looked up in it.
    let all_files: Vec<PathBuf> = project_files(root)
        .filter_map(|entry| entry.path().strip_prefix(root).ok().map(Path::to_path_buf))
        .collect();
    let modules = crate::map::layout::find_modules(root, &all_files);
    let resolver = crate::map::layout::Resolver::new(root, &all_files, &modules);
    for flow in &mut data_flows {
        let from = flow.from.strip_prefix(root).unwrap_or(&flow.from);
        // Resolve using the importing file's language, not the project language.
        // Per-file detection was already applied during extraction; use it here too.
        let file_language = Language::detect(from).unwrap_or(project.language);
        flow.resolved = resolver.resolve(from, &flow.to.to_string_lossy(), file_language);
    }
    let mut docs: Vec<PathBuf> = all_files
        .iter()
        .filter(|f| {
            f.extension()
                .and_then(|e| e.to_str())
                .is_some_and(crate::map::outline::is_doc)
        })
        .cloned()
        .collect();
    docs.sort();
    let mut files: Vec<PathBuf> = all_files
        .into_iter()
        .filter(|f| {
            f.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| CODE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        })
        .collect();
    files.sort();

    let mut symbols = index_symbols(root, &files);
    symbols.extend(
        index_symbols(root, &docs)
            .into_iter()
            .filter(|s| matches!(s.kind.as_str(), "h1" | "h2" | "h3")),
    );

    let mut hot_zones = Vec::new();
    let mut warm_zones = Vec::new();
    let mut cold_zones = Vec::new();
    let mut frozen_zones = Vec::new();

    for zone in zones {
        match zone.frequency {
            ZoneFrequency::Hot => hot_zones.push(zone),
            ZoneFrequency::Warm => warm_zones.push(zone),
            ZoneFrequency::Cold => cold_zones.push(zone),
            ZoneFrequency::Frozen => frozen_zones.push(zone),
        }
    }

    // Warm zones go into hot_zones for priority reading
    hot_zones.extend(warm_zones);

    // Most changed first, so "the top N" really is the most active files.
    for zones in [&mut hot_zones, &mut cold_zones, &mut frozen_zones] {
        zones.sort_by(|a, b| {
            b.change_count
                .cmp(&a.change_count)
                .then_with(|| a.path.cmp(&b.path))
        });
    }

    Ok(Fingerprint {
        project_id: project.id,
        entry_points,
        data_flows,
        hot_zones,
        cold_zones,
        frozen_zones,
        language: project.language,
        generated_at: Utc::now(),
        files,
        modules,
        head: current_head(root),
        symbols,
        docs,
        format: FINGERPRINT_FORMAT,
    })
}

/// Current fingerprint layout. Bump it when a scan starts recording something
/// tools depend on, so fingerprints stored by older versions are regenerated.
pub const FINGERPRINT_FORMAT: u32 = 3;

/// Declarations of every code file. Files that cannot be read are skipped.
fn index_symbols(root: &Path, files: &[PathBuf]) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for file in files {
        let Some(ext) = file.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let Ok(content) = std::fs::read_to_string(root.join(file)) else {
            continue;
        };
        symbols.extend(
            crate::map::outline::declarations(&content, ext)
                .into_iter()
                .map(|d| Symbol {
                    name: d.name,
                    kind: d.kind,
                    file: file.clone(),
                    line: d.start,
                }),
        );
    }
    symbols
}

/// Extensions of every language Cogito knows, so a polyglot repository's map
/// shows all of its code and not only the main language's.
const CODE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "py", "go", "java", "cs", "rb",
    "php", "kt", "kts", "swift", "vue", "svelte", "sql", "prisma", "graphql", "proto",
];

/// The commit `HEAD` points at, if `path` is inside a git repository.
pub fn current_head(path: &Path) -> Option<String> {
    let repo = git2::Repository::discover(path).ok()?;
    let commit = repo.head().ok()?.peel_to_commit().ok()?;
    Some(commit.id().to_string())
}

/// Find entry points based on language conventions
fn find_entry_points(path: &Path, language: &Language) -> Result<Vec<PathBuf>> {
    let mut entry_points = Vec::new();

    let patterns: &[&str] = match language {
        Language::Rust => &["**/main.rs", "**/lib.rs", "**/mod.rs", "**/bin/*.rs"],
        Language::TypeScript | Language::JavaScript => &[
            "**/index.ts",
            "**/index.js",
            "**/main.ts",
            "**/main.js",
            "**/app.ts",
            "**/app.tsx",
            "**/server.ts",
            "**/server.js",
            "**/api/index.ts",
        ],
        Language::Python => &[
            "**/__main__.py",
            "**/main.py",
            "**/__init__.py",
            "**/app.py",
            "**/manage.py",
            "**/wsgi.py",
        ],
        Language::Go => &["**/main.go", "**/cmd/*.go"],
        Language::Java => &["**/Main.java", "**/Application.java"],
        Language::CSharp => &["**/Program.cs", "**/Startup.cs", "**/Application.cs"],
        Language::Ruby => &["**/config.ru", "**/Rakefile", "**/bin/*"],
        Language::Php => &["**/index.php", "**/app.php"],
        Language::Kotlin => &["**/Main.kt", "**/Application.kt", "**/main.kt"],
        Language::Swift => &["**/main.swift", "**/App.swift", "**/Package.swift"],
        Language::Unknown => &[],
    };

    // Patterns are matched against paths relative to the project while walking
    // it, instead of with `glob::glob`: that follows symlinks and descends into
    // `node_modules`, where pnpm/yarn workspaces link packages into each other,
    // so a monorepo sends it round a cycle until the OS reports ELOOP.
    let matchers = patterns
        .iter()
        .map(|pattern| {
            glob::Pattern::new(pattern).with_context(|| format!("Invalid glob pattern: {pattern}"))
        })
        .collect::<Result<Vec<_>>>()?;
    // `*` stays within one path segment, as it did with `glob::glob`.
    let options = glob::MatchOptions {
        require_literal_separator: true,
        ..glob::MatchOptions::default()
    };

    for entry in project_files(path) {
        let Ok(relative) = entry.path().strip_prefix(path) else {
            continue;
        };
        if matchers
            .iter()
            .any(|m| m.matches_path_with(relative, options))
        {
            entry_points.push(entry.into_path());
        }
    }

    // Deduplicate
    entry_points.sort();
    entry_points.dedup();

    Ok(entry_points)
}

/// Analyze data flows between modules via import/require analysis.
/// Uses per-file language detection so polyglot repositories extract imports
/// from every file regardless of the project's detected primary language.
fn analyze_data_flows(path: &Path, project_language: &Language) -> Result<Vec<DataFlow>> {
    let mut flows = Vec::new();

    for entry in project_files(path) {
        let file_path = entry.path().to_path_buf();

        // Detect language per-file, falling back to project language for unknown extensions
        let language = Language::detect(&file_path).unwrap_or(*project_language);

        let Some(ext) = file_path.extension() else {
            continue;
        };
        if !is_code_file(ext, &language) {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&file_path) else {
            continue;
        };

        flows.extend(extract_imports(&file_path, &content, &language)?);
    }

    Ok(flows)
}

/// Dependency lockfiles, generated by package managers.
const LOCKFILES: &[&str] = &[
    "pnpm-lock.yaml",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
    "Cargo.lock",
    "poetry.lock",
    "Pipfile.lock",
    "uv.lock",
    "composer.lock",
    "Gemfile.lock",
    "go.sum",
    "flake.lock",
];

/// Directories that hold dependencies, build output or VCS data, never the
/// project's own code.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".git",
    ".next",
    "vendor",
    "venv",
    "__pycache__",
    ".venv",
    "out",
    "coverage",
];

/// Whether `path` lies inside one of the ignored directories.
#[cfg(test)]
fn should_skip(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| SKIP_DIRS.contains(&s))
    })
}

/// The project's own files.
///
/// Symlinks are not followed, and ignored directories are pruned rather than
/// walked and filtered afterwards: a JS monorepo's `node_modules` can hold more
/// files than the project, and its workspace links form cycles. In a git
/// repository, paths `.gitignore` excludes (generated clients, build output)
/// are skipped too. Entries that cannot be read are skipped, so one bad path
/// never fails the whole scan.
fn project_files(path: &Path) -> impl Iterator<Item = walkdir::DirEntry> {
    let repo = git2::Repository::discover(path).ok();
    let workdir = repo
        .as_ref()
        .and_then(|r| r.workdir())
        .and_then(|w| w.canonicalize().ok());
    let root = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |entry| {
            // The root is always walked, whatever it is called.
            if entry.depth() == 0 {
                return true;
            }
            if entry.file_type().is_dir()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| SKIP_DIRS.contains(&name))
            {
                return false;
            }
            !is_git_ignored(repo.as_ref(), workdir.as_deref(), &root, path, entry.path())
        })
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
}

/// Whether git ignores `entry` (a path under `walk_root`).
fn is_git_ignored(
    repo: Option<&git2::Repository>,
    workdir: Option<&Path>,
    root: &Path,
    walk_root: &Path,
    entry: &Path,
) -> bool {
    let (Some(repo), Some(workdir)) = (repo, workdir) else {
        return false;
    };
    let Ok(below_root) = entry.strip_prefix(walk_root) else {
        return false;
    };
    let Ok(in_repo) = root
        .join(below_root)
        .strip_prefix(workdir)
        .map(Path::to_path_buf)
    else {
        return false;
    };
    repo.is_path_ignored(&in_repo).unwrap_or(false)
}

/// Check if file extension matches language
fn is_code_file(ext: &std::ffi::OsStr, language: &Language) -> bool {
    let ext_str = ext.to_string_lossy().to_lowercase();
    language.extensions().iter().any(|e| *e == ext_str)
}

/// Extract imports from file content
fn extract_imports(file_path: &Path, content: &str, language: &Language) -> Result<Vec<DataFlow>> {
    let mut flows = Vec::new();

    match language {
        Language::Rust => {
            let re = regex::Regex::new(r"use\s+((?:crate|self|super)::[a-zA-Z0-9_:]+)")?;
            for cap in re.captures_iter(content) {
                if let Some(name) = cap.get(1) {
                    flows.push(DataFlow {
                        from: file_path.to_path_buf(),
                        to: PathBuf::from(name.as_str()),
                        flow_type: FlowType::Import,
                        resolved: None,
                    });
                }
            }
        }
        Language::TypeScript | Language::JavaScript => {
            // `import … from`, side-effect `import '…'`, `export … from`,
            // `require('…')` and dynamic `import('…')`.
            let import_re = regex::Regex::new(
                r#"(?:(?:import|export)\s+(?:[^'";]+?\s+from\s+)?|(?:require|import)\(\s*)["']([^"']+)["']"#,
            )?;
            for cap in import_re.captures_iter(content) {
                if let Some(module) = cap.get(1) {
                    flows.push(DataFlow {
                        from: file_path.to_path_buf(),
                        to: PathBuf::from(module.as_str()),
                        flow_type: FlowType::Import,
                        resolved: None,
                    });
                }
            }
        }
        Language::Python => {
            // `import pkg` and `import pkg.sub`
            let import_re = regex::Regex::new(r"^\s*import\s+([a-zA-Z0-9_.]+)")?;
            // `from pkg import x`, `from .sibling import x`, `from ..parent import x`
            // Allow leading dots for relative imports.
            let from_re =
                regex::Regex::new(r"^\s*from\s+([.a-zA-Z0-9_]+)\s+import\s+([a-zA-Z0-9_*, ]+)")?;

            for line in content.lines() {
                if let Some(cap) = import_re.captures(line)
                    && let Some(module) = cap.get(1)
                {
                    flows.push(DataFlow {
                        from: file_path.to_path_buf(),
                        to: PathBuf::from(module.as_str()),
                        flow_type: FlowType::Import,
                        resolved: None,
                    });
                }
                if let Some(cap) = from_re.captures(line)
                    && let Some(module) = cap.get(1)
                {
                    flows.push(DataFlow {
                        from: file_path.to_path_buf(),
                        to: PathBuf::from(module.as_str()),
                        flow_type: FlowType::Import,
                        resolved: None,
                    });
                }
            }
        }
        Language::Go => {
            let import_re = regex::Regex::new(r#""([a-zA-Z0-9_/.\-]+)""#)?;
            let mut in_import_block = false;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("import (") {
                    in_import_block = true;
                    continue;
                }
                if in_import_block {
                    if trimmed == ")" {
                        in_import_block = false;
                        continue;
                    }
                    if let Some(cap) = import_re.captures(trimmed)
                        && let Some(module) = cap.get(1)
                    {
                        flows.push(DataFlow {
                            from: file_path.to_path_buf(),
                            to: PathBuf::from(module.as_str()),
                            flow_type: FlowType::Import,
                            resolved: None,
                        });
                    }
                } else if trimmed.starts_with("import ")
                    && let Some(cap) = import_re.captures(trimmed)
                    && let Some(module) = cap.get(1)
                {
                    flows.push(DataFlow {
                        from: file_path.to_path_buf(),
                        to: PathBuf::from(module.as_str()),
                        flow_type: FlowType::Import,
                        resolved: None,
                    });
                }
            }
        }
        Language::Java => {
            // `import a.b.C;`, `import static a.b.C.member;`, `import a.b.*;`
            // Strip trailing `.member` or `.*` down to the type or package path.
            for line in content.lines() {
                let trimmed = line.trim();
                // Wildcard import: `import a.b.*;`
                if let Some(import_content) = trimmed.strip_prefix("import ") {
                    let is_static = import_content.starts_with("static ");
                    let content = if is_static {
                        import_content.strip_prefix("static ").unwrap()
                    } else {
                        import_content
                    };
                    // Strip trailing `;`
                    let Some(without_semicolon) = content.strip_suffix(';') else {
                        continue;
                    };
                    if let Some(package) = without_semicolon.strip_suffix(".*") {
                        // Wildcard import: `a.b.*` → keep `a.b`
                        flows.push(DataFlow {
                            from: file_path.to_path_buf(),
                            to: PathBuf::from(package),
                            flow_type: FlowType::Import,
                            resolved: None,
                        });
                    } else if is_static {
                        // Static import: `a.b.C.member` → keep `a.b.C`
                        if let Some((pkg, _member)) = without_semicolon.rsplit_once('.') {
                            flows.push(DataFlow {
                                from: file_path.to_path_buf(),
                                to: PathBuf::from(pkg),
                                flow_type: FlowType::Import,
                                resolved: None,
                            });
                        }
                    } else {
                        // Plain import: `a.b.C;` → keep `a.b.C`
                        flows.push(DataFlow {
                            from: file_path.to_path_buf(),
                            to: PathBuf::from(without_semicolon),
                            flow_type: FlowType::Import,
                            resolved: None,
                        });
                    }
                }
            }
        }
        _ => {}
    }

    Ok(flows)
}

/// Detect all code zones (hot, warm, cold, frozen) via git history
pub async fn detect_zones(project: &Project) -> Result<Vec<Zone>> {
    detect_zones_path(&project.path)
}

/// Detect zones from a path
fn detect_zones_path(path: &Path) -> Result<Vec<Zone>> {
    let Ok(repo) = git2::Repository::discover(path) else {
        return Ok(Vec::new());
    };

    let mut zone_map: HashMap<PathBuf, ZoneStats> = HashMap::new();
    let now = Utc::now();

    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;

    // A file's activity is the commits that *changed* it, so each commit is
    // diffed against its parent. Walking each commit's whole tree instead would
    // count every commit in which the file merely exists, and date every file
    // to HEAD.
    for oid in revwalk {
        let Ok(oid) = oid else { continue };
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        // A merge repeats changes its parents already made; counting it too
        // would double every change that came in through a branch.
        if commit.parent_count() > 1 {
            continue;
        }
        let Ok(tree) = commit.tree() else {
            continue;
        };
        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
        let diff = match repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None) {
            Ok(diff) => diff,
            Err(e) => {
                tracing::debug!("[cogito] Diff error: {}", e);
                continue;
            }
        };

        let commit_time =
            DateTime::<Utc>::from_timestamp(commit.time().seconds(), 0).unwrap_or_else(Utc::now);

        for delta in diff.deltas() {
            let Some(changed) = delta.new_file().path().or_else(|| delta.old_file().path()) else {
                continue;
            };
            let stats = zone_map
                .entry(changed.to_path_buf())
                .or_insert_with(ZoneStats::new);
            stats.commit_count += 1;
            if commit_time > stats.last_modified {
                stats.last_modified = commit_time;
            }
        }
    }

    // Lockfiles change with every dependency bump, so they top any activity
    // ranking while saying nothing about where the project's work happens.
    zone_map.retain(|file, _| {
        !file
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| LOCKFILES.contains(&name))
    });

    // Only files that still exist are zones of the project.
    if let Ok(head_tree) = repo.head().and_then(|head| head.peel_to_tree()) {
        zone_map.retain(|file, _| head_tree.get_path(file).is_ok());
    }

    let mut zones = Vec::new();
    for (path, stats) in zone_map {
        let age = now.signed_duration_since(stats.last_modified);
        let frequency = if age > Duration::days(FROZEN_THRESHOLD_DAYS) {
            ZoneFrequency::Frozen
        } else if stats.commit_count > HOT_MIN_COMMITS && age < Duration::days(HOT_THRESHOLD_DAYS) {
            ZoneFrequency::Hot
        } else if stats.commit_count > WARM_MIN_COMMITS && age < Duration::days(WARM_THRESHOLD_DAYS)
        {
            ZoneFrequency::Warm
        } else {
            ZoneFrequency::Cold
        };

        zones.push(Zone {
            path,
            frequency,
            last_modified: stats.last_modified,
            change_count: stats.commit_count,
        });
    }

    Ok(zones)
}

struct ZoneStats {
    commit_count: usize,
    last_modified: DateTime<Utc>,
}

impl ZoneStats {
    fn new() -> Self {
        Self {
            commit_count: 0,
            last_modified: DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_else(Utc::now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_entry_points_rust() {
        let path = PathBuf::from(".");
        let language = Language::Rust;
        let result = find_entry_points(&path, &language).unwrap();
        let _ = result;
    }

    /// A pnpm-style workspace: two packages linked into each other's
    /// `node_modules`, which is a symlink cycle.
    #[cfg(unix)]
    fn monorepo_with_symlink_cycle() -> tempfile::TempDir {
        use std::os::unix::fs::symlink;

        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        for pkg in ["shared", "evidence"] {
            std::fs::create_dir_all(root.join(format!("packages/{pkg}/src"))).unwrap();
            std::fs::create_dir_all(root.join(format!("packages/{pkg}/node_modules/@app")))
                .unwrap();
            std::fs::write(
                root.join(format!("packages/{pkg}/src/index.ts")),
                "import { x } from './util';\n",
            )
            .unwrap();
        }
        symlink(
            root.join("packages/evidence"),
            root.join("packages/shared/node_modules/@app/evidence"),
        )
        .unwrap();
        symlink(
            root.join("packages/shared"),
            root.join("packages/evidence/node_modules/@app/shared"),
        )
        .unwrap();
        // Dependency code must never be reported as the project's.
        std::fs::create_dir_all(root.join("node_modules/lib")).unwrap();
        std::fs::write(root.join("node_modules/lib/index.ts"), "export {};\n").unwrap();
        std::fs::write(root.join("index.ts"), "import a from 'a';\n").unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn test_entry_points_survive_workspace_symlink_cycles() {
        let dir = monorepo_with_symlink_cycle();
        let root = dir.path();

        let found: Vec<_> = find_entry_points(root, &Language::TypeScript)
            .expect("a symlink cycle in node_modules must not fail the scan")
            .into_iter()
            .map(|p| p.strip_prefix(root).unwrap().to_path_buf())
            .collect();

        assert_eq!(
            found,
            vec![
                PathBuf::from("index.ts"),
                PathBuf::from("packages/evidence/src/index.ts"),
                PathBuf::from("packages/shared/src/index.ts"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_data_flows_skip_node_modules_and_cycles() {
        let dir = monorepo_with_symlink_cycle();
        let flows = analyze_data_flows(dir.path(), &Language::TypeScript).unwrap();
        assert_eq!(flows.len(), 3, "one import per project file: {flows:?}");
        assert!(
            flows
                .iter()
                .all(|f| !f.from.components().any(|c| c.as_os_str() == "node_modules")),
            "no flow may come from node_modules: {flows:?}"
        );
    }

    #[test]
    fn test_entry_point_patterns_keep_star_within_one_segment() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("cmd/tool/nested")).unwrap();
        std::fs::write(root.join("cmd/run.go"), "package main\n").unwrap();
        std::fs::write(root.join("cmd/tool/nested/deep.go"), "package main\n").unwrap();

        let found: Vec<_> = find_entry_points(root, &Language::Go)
            .unwrap()
            .into_iter()
            .map(|p| p.strip_prefix(root).unwrap().to_path_buf())
            .collect();
        assert_eq!(found, vec![PathBuf::from("cmd/run.go")]);
    }

    fn git(root: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn test_gitignored_files_are_not_project_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        std::fs::create_dir_all(root.join("db/src/generated")).unwrap();
        std::fs::write(root.join("db/.gitignore"), "**/src/generated\n").unwrap();
        std::fs::write(root.join("db/src/index.ts"), "export {};\n").unwrap();
        std::fs::write(root.join("db/src/generated/client.ts"), "export {};\n").unwrap();

        let files: Vec<PathBuf> = project_files(root)
            .map(|e| e.path().strip_prefix(root).unwrap().to_path_buf())
            .filter(|p| p.extension().is_some_and(|e| e == "ts"))
            .collect();
        assert_eq!(files, vec![PathBuf::from("db/src/index.ts")]);
    }

    #[test]
    fn test_zone_counts_are_commits_that_changed_the_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);

        std::fs::write(root.join("busy.ts"), "1").unwrap();
        std::fs::write(root.join("quiet.ts"), "1").unwrap();
        std::fs::write(root.join("gone.ts"), "1").unwrap();
        std::fs::write(root.join("pnpm-lock.yaml"), "1").unwrap();
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "init"]);
        for n in 2..=4 {
            std::fs::write(root.join("busy.ts"), n.to_string()).unwrap();
            git(root, &["commit", "-q", "-am", "touch busy"]);
        }
        std::fs::remove_file(root.join("gone.ts")).unwrap();
        git(root, &["commit", "-q", "-am", "remove gone"]);

        let zones = detect_zones_path(root).unwrap();
        let count = |name: &str| {
            zones
                .iter()
                .find(|z| z.path == Path::new(name))
                .map(|z| z.change_count)
        };
        // quiet.ts exists in all five commits but changed in one.
        assert_eq!(count("busy.ts"), Some(4));
        assert_eq!(count("pnpm-lock.yaml"), None, "lockfiles are not zones");
        assert_eq!(count("quiet.ts"), Some(1));
        assert_eq!(count("gone.ts"), None, "deleted files are not zones");
    }

    #[test]
    fn test_is_code_file() {
        let ext = std::ffi::OsStr::new("rs");
        assert!(is_code_file(ext, &Language::Rust));
        let ext = std::ffi::OsStr::new("py");
        assert!(!is_code_file(ext, &Language::Rust));
    }

    #[test]
    fn test_should_skip() {
        let skip_path = PathBuf::from("/project/node_modules/foo.js");
        assert!(should_skip(&skip_path));
        let keep_path = PathBuf::from("/project/src/main.rs");
        assert!(!should_skip(&keep_path));
    }
}
