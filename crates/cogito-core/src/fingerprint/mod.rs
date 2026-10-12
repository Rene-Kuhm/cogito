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
    let entry_points = find_entry_points(&project.path, &project.language)?;
    let data_flows = analyze_data_flows(&project.path, &project.language)?;
    let zones = detect_zones_path(&project.path)?;

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

    Ok(Fingerprint {
        project_id: project.id,
        entry_points,
        data_flows,
        hot_zones,
        cold_zones,
        frozen_zones,
        language: project.language,
        generated_at: Utc::now(),
    })
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
        Language::Unknown => &[],
    };

    for pattern in patterns {
        for entry in glob::glob(&path.join(pattern).to_string_lossy())
            .with_context(|| format!("Invalid glob pattern: {pattern}"))?
        {
            let entry = entry?;
            if entry.is_file() {
                entry_points.push(entry);
            }
        }
    }

    // Deduplicate
    entry_points.sort();
    entry_points.dedup();

    Ok(entry_points)
}

/// Analyze data flows between modules via import/require analysis
fn analyze_data_flows(path: &Path, language: &Language) -> Result<Vec<DataFlow>> {
    let mut flows = Vec::new();

    for entry in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let file_path = entry.path().to_path_buf();

        if should_skip(&file_path) {
            continue;
        }

        let Some(ext) = file_path.extension() else {
            continue;
        };
        if !is_code_file(ext, language) {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&file_path) else {
            continue;
        };

        flows.extend(extract_imports(&file_path, &content, language)?);
    }

    Ok(flows)
}

/// Skip common ignored directories
fn should_skip(path: &Path) -> bool {
    let skip_dirs = [
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

    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| skip_dirs.contains(&s))
            .unwrap_or(false)
    })
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
            let re = regex::Regex::new(r"use\s+(crate|self|super)::([a-zA-Z0-9_::]+)")?;
            for cap in re.captures_iter(content) {
                if let Some(name) = cap.get(2) {
                    flows.push(DataFlow {
                        from: file_path.to_path_buf(),
                        to: PathBuf::from(name.as_str()),
                        flow_type: FlowType::Import,
                    });
                }
            }
        }
        Language::TypeScript | Language::JavaScript => {
            let import_re = regex::Regex::new(
                r#"(?:import\s+(?:[^'"]+\s+from\s+)?|require\(\s*)["']([^"']+)["']"#,
            )?;
            for cap in import_re.captures_iter(content) {
                if let Some(module) = cap.get(1) {
                    flows.push(DataFlow {
                        from: file_path.to_path_buf(),
                        to: PathBuf::from(module.as_str()),
                        flow_type: FlowType::Import,
                    });
                }
            }
        }
        Language::Python => {
            let import_re = regex::Regex::new(r"^\s*import\s+([a-zA-Z0-9_.]+)")?;
            let from_re =
                regex::Regex::new(r"^\s*from\s+([a-zA-Z0-9_.]+)\s+import\s+([a-zA-Z0-9_*, ]+)")?;

            for line in content.lines() {
                if let Some(cap) = import_re.captures(line)
                    && let Some(module) = cap.get(1)
                {
                    flows.push(DataFlow {
                        from: file_path.to_path_buf(),
                        to: PathBuf::from(module.as_str()),
                        flow_type: FlowType::Import,
                    });
                }
                if let Some(cap) = from_re.captures(line)
                    && let Some(module) = cap.get(1)
                {
                    flows.push(DataFlow {
                        from: file_path.to_path_buf(),
                        to: PathBuf::from(module.as_str()),
                        flow_type: FlowType::Import,
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
                    });
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

    for oid in revwalk {
        let Ok(oid) = oid else { continue };
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        let Ok(tree) = commit.tree() else {
            continue;
        };

        let commit_time =
            DateTime::<Utc>::from_timestamp(commit.time().seconds(), 0).unwrap_or_else(Utc::now);

        let walk_result = tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
            let Some(entry_name) = entry.name() else {
                return git2::TreeWalkResult::Ok;
            };
            let rel_path = if root.is_empty() {
                PathBuf::from(entry_name)
            } else {
                PathBuf::from(root).join(entry_name)
            };
            let Ok(obj) = entry.to_object(&repo) else {
                return git2::TreeWalkResult::Ok;
            };
            if obj.as_blob().is_some() {
                let stats = zone_map.entry(rel_path).or_insert(ZoneStats::new());
                stats.commit_count += 1;
                if commit_time > stats.last_modified {
                    stats.last_modified = commit_time;
                }
            }
            git2::TreeWalkResult::Ok
        });

        if let Err(e) = walk_result {
            tracing::debug!("[cogito] Walk error: {}", e);
        }
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
