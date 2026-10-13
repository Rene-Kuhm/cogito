//! The project map: what an agent needs to find its way around a codebase
//! without reading it.
//!
//! Everything here is derived from a stored [`Fingerprint`], so building a
//! map, listing a file's neighbours or searching paths reads no source files.
//! The renderers are deliberately terse: their output is spent as agent
//! context, so every line has to save more reading than it costs.

pub(crate) mod layout;

use crate::types::{Fingerprint, ModuleInfo};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// Lines of hot files shown per module, and in the project-wide list.
const HOT_PER_MODULE: usize = 3;
const HOT_OVERALL: usize = 10;
/// Cap on each list in `related` and `search` output.
const LIST_LIMIT: usize = 25;
/// A file changed once is not "hot"; listing it would only spend tokens.
const MIN_HOT_CHANGES: usize = 2;

/// One module (a sub-project with a manifest, or a top-level directory).
#[derive(Debug, Clone)]
pub struct ModuleSummary {
    pub path: PathBuf,
    pub name: String,
    pub files: usize,
    pub entry: Option<PathBuf>,
    /// Most changed files, relative to the module, with their change counts.
    pub hot: Vec<(PathBuf, usize)>,
    /// Other modules this one imports from.
    pub uses: Vec<PathBuf>,
    /// How many other modules import from this one.
    pub used_by: usize,
}

/// A compact overview of a project.
#[derive(Debug, Clone)]
pub struct ProjectMap {
    pub name: String,
    pub fingerprint_language: String,
    pub generated_at: String,
    pub head: Option<String>,
    pub total_files: usize,
    pub modules: Vec<ModuleSummary>,
    pub hot_files: Vec<(PathBuf, usize)>,
}

/// Paths in a fingerprint relative to `root`. Older fingerprints stored some
/// absolute paths; this accepts both.
fn rel(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

/// Modules to group files by. Without any manifest below the root, the
/// top-level directories stand in for modules.
fn effective_modules(fp: &Fingerprint) -> Vec<ModuleInfo> {
    if !fp.modules.is_empty() {
        return fp.modules.clone();
    }
    let dirs: BTreeSet<PathBuf> = fp
        .files
        .iter()
        .filter(|f| f.components().count() > 1)
        .filter_map(|f| f.components().next())
        .map(|c| PathBuf::from(c.as_os_str()))
        .collect();
    dirs.into_iter()
        .map(|path| ModuleInfo {
            name: path.display().to_string(),
            path,
            manifest: String::new(),
        })
        .collect()
}

/// The module a file belongs to: the deepest module containing it, or the
/// root (`""`) when none does.
fn module_of<'a>(modules: &'a [ModuleInfo], file: &Path) -> Option<&'a ModuleInfo> {
    modules
        .iter()
        .filter(|m| file.starts_with(&m.path))
        .max_by_key(|m| m.path.components().count())
}

/// Change count of every file the fingerprint knows about.
fn change_counts(root: &Path, fp: &Fingerprint) -> HashMap<PathBuf, usize> {
    fp.hot_zones
        .iter()
        .chain(&fp.cold_zones)
        .chain(&fp.frozen_zones)
        .map(|z| (rel(root, &z.path), z.change_count))
        .collect()
}

/// Build the map of a scanned project.
pub fn build(root: &Path, name: &str, fp: &Fingerprint) -> ProjectMap {
    let modules = effective_modules(fp);
    let changes = change_counts(root, fp);
    // Activity rankings list code only: manifests, lockfiles, docs and CI
    // config change often but are not where an agent goes to read code.
    let code: std::collections::HashSet<&PathBuf> = fp.files.iter().collect();
    let key = |file: &Path| module_of(&modules, file).map(|m| m.path.clone());

    let mut files_per: HashMap<Option<PathBuf>, usize> = HashMap::new();
    for file in &fp.files {
        *files_per.entry(key(file)).or_default() += 1;
    }

    // Module → modules it imports from.
    let mut uses: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
    for flow in &fp.data_flows {
        let Some(target) = &flow.resolved else {
            continue;
        };
        let (Some(from), Some(to)) = (key(&rel(root, &flow.from)), key(target)) else {
            continue;
        };
        if from != to {
            uses.entry(from).or_default().insert(to);
        }
    }
    let mut used_by: HashMap<PathBuf, usize> = HashMap::new();
    for targets in uses.values() {
        for t in targets {
            *used_by.entry(t.clone()).or_default() += 1;
        }
    }

    let entries: Vec<PathBuf> = fp.entry_points.iter().map(|e| rel(root, e)).collect();
    let mut summaries: Vec<ModuleSummary> = modules
        .iter()
        .map(|m| {
            let mut hot: Vec<(PathBuf, usize)> = changes
                .iter()
                .filter(|(f, c)| **c >= MIN_HOT_CHANGES && code.contains(*f))
                .filter(|(f, _)| module_of(&modules, f).is_some_and(|o| o.path == m.path))
                .map(|(f, c)| (rel(&m.path, f), *c))
                .collect();
            hot.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            hot.truncate(HOT_PER_MODULE);

            // A module's front door sits at its root or in `src/`. A deeper
            // match (`lib/auth/server.ts`) is just a file with a common name,
            // so none is shown rather than a misleading one.
            let entry = entries
                .iter()
                .filter(|e| module_of(&modules, e).is_some_and(|o| o.path == m.path))
                .map(|e| rel(&m.path, e))
                .filter(|e| {
                    let depth = e.components().count();
                    depth == 1 || (depth == 2 && e.starts_with("src"))
                })
                .min_by_key(|e| (e.components().count(), e.as_os_str().len()));

            ModuleSummary {
                path: m.path.clone(),
                name: m.name.clone(),
                files: files_per.get(&Some(m.path.clone())).copied().unwrap_or(0),
                entry,
                hot,
                uses: uses
                    .get(&m.path)
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default(),
                used_by: used_by.get(&m.path).copied().unwrap_or(0),
            }
        })
        .filter(|m| m.files > 0)
        .collect();
    summaries.sort_by(|a, b| a.path.cmp(&b.path));

    let mut hot_files: Vec<(PathBuf, usize)> = fp
        .hot_zones
        .iter()
        .map(|z| (rel(root, &z.path), z.change_count))
        .filter(|(f, _)| code.contains(f))
        .collect();
    hot_files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    hot_files.truncate(HOT_OVERALL);

    ProjectMap {
        name: name.to_string(),
        fingerprint_language: fp.language.to_string(),
        generated_at: fp.generated_at.format("%Y-%m-%d %H:%M UTC").to_string(),
        head: fp.head.as_ref().map(|h| h.chars().take(7).collect()),
        total_files: fp.files.len(),
        modules: summaries,
        hot_files,
    }
}

fn join_counts(items: &[(PathBuf, usize)]) -> String {
    items
        .iter()
        .map(|(p, c)| format!("{} ({c})", p.display()))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Render the map as compact text for an agent.
pub fn render(map: &ProjectMap) -> String {
    let mut out = format!(
        "{} · {} · {} code files · {} modules · scanned {}{}\n",
        map.name,
        map.fingerprint_language,
        map.total_files,
        map.modules.len(),
        map.generated_at,
        map.head
            .as_ref()
            .map(|h| format!(" at {h}"))
            .unwrap_or_default(),
    );

    for m in &map.modules {
        out.push('\n');
        let path = m.path.display().to_string();
        let mut line = path.clone();
        if !m.name.is_empty() && m.name != path && !path.ends_with(&m.name) {
            line.push_str(&format!(" ({})", m.name));
        }
        line.push_str(&format!(" · {} files", m.files));
        if let Some(entry) = &m.entry {
            line.push_str(&format!(" · entry {}", entry.display()));
        }
        if m.used_by > 0 {
            line.push_str(&format!(" · used by {}", m.used_by));
        }
        out.push_str(&line);
        out.push('\n');
        if !m.hot.is_empty() {
            out.push_str(&format!("  hot: {}\n", join_counts(&m.hot)));
        }
        if !m.uses.is_empty() {
            let uses: Vec<String> = m.uses.iter().map(|u| u.display().to_string()).collect();
            out.push_str(&format!("  uses: {}\n", uses.join(", ")));
        }
    }

    if !map.hot_files.is_empty() {
        out.push_str(&format!(
            "\nMost changed: {}\n",
            join_counts(&map.hot_files)
        ));
    }
    out.push_str(
        "\nNext: related_files(file) for a file's imports and importers; \
         search_code(query) to find files by path, module or tag.\n",
    );
    out
}

/// Find the file `query` names: an exact project path, or a unique suffix
/// such as `auth/server.ts`.
pub fn find_file(root: &Path, fp: &Fingerprint, query: &str) -> Result<PathBuf, String> {
    let wanted = rel(root, Path::new(query.trim()));
    if fp.files.contains(&wanted) {
        return Ok(wanted);
    }
    let matches: Vec<&PathBuf> = fp.files.iter().filter(|f| f.ends_with(&wanted)).collect();
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!(
            "{} is not a code file of this project (try search_code)",
            wanted.display()
        )),
        many => Err(format!(
            "{} matches {} files, be more specific: {}",
            wanted.display(),
            many.len(),
            many.iter()
                .take(8)
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// A file's neighbourhood: what it imports and what imports it.
pub fn render_related(root: &Path, fp: &Fingerprint, file: &Path) -> String {
    let modules = effective_modules(fp);
    let changes = change_counts(root, fp);

    let mut imports = BTreeSet::new();
    let mut external = BTreeSet::new();
    let mut importers = BTreeSet::new();
    for flow in &fp.data_flows {
        let from = rel(root, &flow.from);
        if from == file {
            match &flow.resolved {
                Some(target) => {
                    imports.insert(target.clone());
                }
                None => {
                    external.insert(flow.to.display().to_string());
                }
            }
        }
        if flow.resolved.as_deref() == Some(file) && from != file {
            importers.insert(from);
        }
    }

    let mut out = file.display().to_string();
    if let Some(m) = module_of(&modules, file) {
        out.push_str(&format!(" · module {}", m.path.display()));
    }
    if let Some(c) = changes.get(file) {
        out.push_str(&format!(" · {c} changes"));
    }
    out.push('\n');

    let list = |title: &str, items: Vec<String>, out: &mut String| {
        if items.is_empty() {
            return;
        }
        out.push_str(&format!("\n{title} ({}):\n", items.len()));
        for item in items.iter().take(LIST_LIMIT) {
            out.push_str(&format!("  {item}\n"));
        }
        if items.len() > LIST_LIMIT {
            out.push_str(&format!("  … {} more\n", items.len() - LIST_LIMIT));
        }
    };
    let shown = |set: BTreeSet<PathBuf>| set.into_iter().map(|p| p.display().to_string()).collect();
    list("Imports", shown(imports), &mut out);
    list("Imported by", shown(importers), &mut out);
    list("External", external.into_iter().collect(), &mut out);
    out
}

/// Whether a lowercase path is a test file or lives in a test directory.
fn is_test(path: &str) -> bool {
    path.split('/').any(|part| {
        matches!(
            part,
            "test" | "tests" | "__tests__" | "e2e" | "spec" | "specs"
        ) || part.contains(".test.")
            || part.contains(".spec.")
            || part.ends_with("_test.go")
            || part.starts_with("test_")
    })
}

/// Files and modules whose path or name matches `query`, best first.
///
/// Every whitespace-separated term is matched case-insensitively against the
/// path; a file ranks by how many terms it matches, then by how often it
/// changed.
pub fn search_paths(root: &Path, fp: &Fingerprint, query: &str) -> (Vec<String>, Vec<String>) {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|t| t.len() >= 2)
        .collect();
    if terms.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let changes = change_counts(root, fp);

    let mut scored: Vec<(usize, bool, usize, &PathBuf)> = fp
        .files
        .iter()
        .filter_map(|f| {
            let path = f.display().to_string().to_lowercase();
            let hits = terms.iter().filter(|t| path.contains(t.as_str())).count();
            (hits > 0).then(|| {
                (
                    hits,
                    is_test(&path),
                    changes.get(f).copied().unwrap_or(0),
                    f,
                )
            })
        })
        .collect();
    // More terms matched first; source before tests; then the busier file.
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.3.cmp(b.3))
    });
    let files = scored
        .iter()
        .take(LIST_LIMIT)
        .map(|(_, _, c, f)| {
            if *c > 0 {
                format!("{} ({c} changes)", f.display())
            } else {
                f.display().to_string()
            }
        })
        .collect();

    let modules = effective_modules(fp)
        .into_iter()
        .filter(|m| {
            let hay = format!("{} {}", m.path.display(), m.name).to_lowercase();
            terms.iter().any(|t| hay.contains(t.as_str()))
        })
        .map(|m| format!("{} ({})", m.path.display(), m.name))
        .collect();
    (files, modules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DataFlow, FlowType, Language, Zone, ZoneFrequency};
    use chrono::Utc;

    fn zone(path: &str, changes: usize) -> Zone {
        Zone {
            path: PathBuf::from(path),
            frequency: ZoneFrequency::Hot,
            last_modified: Utc::now(),
            change_count: changes,
        }
    }

    fn fingerprint() -> Fingerprint {
        let files = [
            "apps/web/lib/auth/server.ts",
            "apps/web/app/page.tsx",
            "apps/web/tests/auth.test.ts",
            "packages/shared/src/index.ts",
        ];
        Fingerprint {
            project_id: uuid::Uuid::nil(),
            entry_points: vec![
                PathBuf::from("apps/web/lib/auth/server.ts"),
                PathBuf::from("packages/shared/src/index.ts"),
            ],
            data_flows: vec![DataFlow {
                from: PathBuf::from("apps/web/lib/auth/server.ts"),
                to: PathBuf::from("@m/shared"),
                flow_type: FlowType::Import,
                resolved: Some(PathBuf::from("packages/shared/src/index.ts")),
            }],
            hot_zones: vec![
                zone("apps/web/lib/auth/server.ts", 9),
                zone("apps/web/tests/auth.test.ts", 12),
                zone("apps/web/package.json", 30),
                zone("apps/web/app/page.tsx", 1),
            ],
            cold_zones: vec![],
            frozen_zones: vec![],
            language: Language::TypeScript,
            generated_at: Utc::now(),
            files: files.iter().map(PathBuf::from).collect(),
            modules: vec![
                ModuleInfo {
                    path: "apps/web".into(),
                    name: "@m/web".into(),
                    manifest: "package.json".into(),
                },
                ModuleInfo {
                    path: "packages/shared".into(),
                    name: "@m/shared".into(),
                    manifest: "package.json".into(),
                },
            ],
            head: None,
        }
    }

    #[test]
    fn test_map_rules() {
        let map = build(Path::new(""), "mono", &fingerprint());
        let web = map
            .modules
            .iter()
            .find(|m| m.path == Path::new("apps/web"))
            .unwrap();
        // lib/auth/server.ts matched an entry-point pattern but sits deep in
        // the module: it is not presented as the module's entry.
        assert_eq!(web.entry, None);
        // Manifests are not hot code, and one change is not hot.
        let hot: Vec<_> = web
            .hot
            .iter()
            .map(|(p, _)| p.display().to_string())
            .collect();
        assert_eq!(hot, ["tests/auth.test.ts", "lib/auth/server.ts"]);
        assert_eq!(web.uses, [PathBuf::from("packages/shared")]);

        let shared = map
            .modules
            .iter()
            .find(|m| m.path == Path::new("packages/shared"))
            .unwrap();
        assert_eq!(shared.entry.as_deref(), Some(Path::new("src/index.ts")));
        assert_eq!(shared.used_by, 1);

        let text = render(&map);
        assert!(!text.contains("package.json"), "{text}");
    }

    #[test]
    fn test_search_ranks_source_before_tests() {
        let (files, modules) = search_paths(Path::new(""), &fingerprint(), "auth");
        assert_eq!(
            files,
            [
                "apps/web/lib/auth/server.ts (9 changes)",
                "apps/web/tests/auth.test.ts (12 changes)"
            ]
        );
        assert!(modules.is_empty());
    }

    #[test]
    fn test_find_file_by_unique_suffix() {
        let fp = fingerprint();
        assert_eq!(
            find_file(Path::new(""), &fp, "auth/server.ts").unwrap(),
            PathBuf::from("apps/web/lib/auth/server.ts")
        );
        assert!(find_file(Path::new(""), &fp, "missing.ts").is_err());
    }
}
