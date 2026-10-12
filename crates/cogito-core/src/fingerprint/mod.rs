//! Code Fingerprint Generator
//!
//! Creates minimal structural snapshots of codebases.

use crate::types::*;
use anyhow::Result;
use chrono::Utc;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Generate fingerprint for a project
pub async fn generate(project: &Project) -> Result<Fingerprint> {
    let mut entry_points = Vec::new();
    let mut data_flows = Vec::new();

    // Find entry points
    entry_points.extend(find_entry_points(&project.path, &project.language)?);

    // Analyze data flows
    data_flows.extend(analyze_data_flows(&project.path, &project.language)?);

    // Detect zones
    let hot_zones = detect_hot_zones(&project.path)?;
    let cold_zones = detect_cold_zones(&project.path)?;
    let frozen_zones = detect_frozen_zones(&project.path)?;

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

/// Find entry points based on language
fn find_entry_points(path: &Path, language: &Language) -> Result<Vec<PathBuf>> {
    let mut entry_points = Vec::new();

    let patterns = match language {
        Language::Rust => vec!["**/main.rs", "**/lib.rs"],
        Language::TypeScript | Language::JavaScript => {
            vec!["**/index.ts", "**/index.js", "**/main.ts", "**/main.js", "**/app.ts"]
        }
        Language::Python => vec!["**/__main__.py", "**/main.py", "**/__init__.py"],
        Language::Go => vec!["**/main.go"],
        Language::Java => vec!["**/Main.java"],
        Language::CSharp => vec!["**/Program.cs"],
        _ => vec![],
    };

    for pattern in patterns {
        for entry in glob::glob(&path.join(pattern).to_string_lossy())?.flatten() {
            entry_points.push(entry);
        }
    }

    Ok(entry_points)
}

/// Analyze data flows between modules
fn analyze_data_flows(path: &Path, language: &Language) -> Result<Vec<DataFlow>> {
    let mut flows = Vec::new();

    for entry in WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let file_path = entry.path().to_path_buf();
        
        if let Some(ext) = file_path.extension()
            && is_code_file(ext, language)
                && let Ok(content) = std::fs::read_to_string(&file_path) {
                    flows.extend(extract_imports(&file_path, &content, language)?);
                }
    }

    Ok(flows)
}

/// Check if file extension matches language
fn is_code_file(ext: &std::ffi::OsStr, language: &Language) -> bool {
    let ext_str = ext.to_string_lossy().to_lowercase();
    language.extensions().iter().any(|e| *e == ext_str)
}

/// Extract imports from file content
fn extract_imports(
    file_path: &Path,
    content: &str,
    language: &Language,
) -> Result<Vec<DataFlow>> {
    let mut flows = Vec::new();

    match language {
        Language::Rust => {
            // Match `use` statements
            let re = regex::Regex::new(r"use\s+([a-zA-Z0-9_::]+)")?;
            for cap in re.captures_iter(content) {
                if let Some(module) = cap.get(1) {
                    flows.push(DataFlow {
                        from: file_path.to_path_buf(),
                        to: PathBuf::from(module.as_str()),
                        flow_type: FlowType::Import,
                    });
                }
            }
        }
        Language::TypeScript | Language::JavaScript => {
            // Match `import` statements
            let re = regex::Regex::new(r#"import\s+(?:\{\s*)?([a-zA-Z0-9_*\s,}]+)(?:\s*\})?\s+from\s+["']([^"']+)["']"#)?;
            for cap in re.captures_iter(content) {
                if let Some(module) = cap.get(2) {
                    flows.push(DataFlow {
                        from: file_path.to_path_buf(),
                        to: PathBuf::from(module.as_str()),
                        flow_type: FlowType::Import,
                    });
                }
            }
        }
        Language::Python => {
            // Match `import` and `from ... import` statements
            let re = regex::Regex::new(r"(?:from\s+([a-zA-Z0-9_.]+)\s+)?import\s+([a-zA-Z0-9_]+)")?;
            for cap in re.captures_iter(content) {
                let module = cap.get(1).or(cap.get(2)).map(|m| m.as_str()).unwrap_or("");
                flows.push(DataFlow {
                    from: file_path.to_path_buf(),
                    to: PathBuf::from(module),
                    flow_type: FlowType::Import,
                });
            }
        }
        _ => {}
    }

    Ok(flows)
}

/// Detect hot zones (frequently changed)
pub async fn detect_zones(project: &Project) -> Result<Vec<Zone>> {
    let hot = detect_hot_zones(&project.path)?;
    let cold = detect_cold_zones(&project.path)?;
    let frozen = detect_frozen_zones(&project.path)?;

    let mut zones = Vec::new();
    zones.extend(hot);
    zones.extend(cold);
    zones.extend(frozen);
    
    Ok(zones)
}

fn detect_hot_zones(_path: &Path) -> Result<Vec<Zone>> {
    // TODO: Implement git history analysis
    // For now, return empty
    Ok(Vec::new())
}

fn detect_cold_zones(_path: &Path) -> Result<Vec<Zone>> {
    // TODO: Implement git history analysis
    Ok(Vec::new())
}

fn detect_frozen_zones(_path: &Path) -> Result<Vec<Zone>> {
    // TODO: Implement git history analysis
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_entry_points_rust() {
        let path = PathBuf::from(".");
        let language = Language::Rust;
        let result = find_entry_points(&path, &language).unwrap();
        // May be empty if no Rust files in current dir
        assert!(result.len() <= 10);
    }

    #[test]
    fn test_extract_imports_typescript() {
        let content = "import { a, b } from './mod';\nimport React from \"react\";";
        let flows = extract_imports(&PathBuf::from("index.ts"), content, &Language::TypeScript).unwrap();
        let targets: Vec<_> = flows.iter().map(|f| f.to.clone()).collect();
        assert_eq!(targets, vec![PathBuf::from("./mod"), PathBuf::from("react")]);
    }

    #[test]
    fn test_is_code_file() {
        use std::ffi::OsStr;
        assert!(is_code_file(OsStr::new("rs"), &Language::Rust));
        assert!(is_code_file(OsStr::new("RS"), &Language::Rust));
        assert!(!is_code_file(OsStr::new("py"), &Language::Rust));
    }
}
