//! Core types for Cogito

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Supported programming languages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Java,
    CSharp,
    Ruby,
    Php,
    Kotlin,
    Swift,
    Unknown,
}

impl Language {
    /// Detect language from a project directory by checking for marker files
    pub fn detect_project(path: &Path) -> anyhow::Result<Self> {
        if path.join("Cargo.toml").exists() {
            return Ok(Language::Rust);
        }
        if path.join("package.json").exists() {
            if has_tsconfig(path) {
                return Ok(Language::TypeScript);
            }
            return Ok(Language::JavaScript);
        }
        if path.join("go.mod").exists() {
            return Ok(Language::Go);
        }
        if path.join("pyproject.toml").exists() || path.join("setup.py").exists() {
            return Ok(Language::Python);
        }
        if path.join("Gemfile").exists() {
            return Ok(Language::Ruby);
        }
        if path.join("composer.json").exists() {
            return Ok(Language::Php);
        }
        if path.join("build.gradle.kts").exists() || path.join("build.gradle.kts").exists() {
            return Ok(Language::Kotlin);
        }
        if path.join("Package.swift").exists() {
            return Ok(Language::Swift);
        }

        // Fallback: try to detect from code files
        for entry in walkdir::WalkDir::new(path)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file()
                && let Some(ext) = entry.path().extension()
            {
                let lang = match ext.to_string_lossy().to_lowercase().as_str() {
                    "rs" => Some(Language::Rust),
                    "ts" | "tsx" => Some(Language::TypeScript),
                    "py" => Some(Language::Python),
                    "go" => Some(Language::Go),
                    _ => None,
                };
                if let Some(l) = lang {
                    return Ok(l);
                }
            }
        }
        Ok(Language::Unknown)
    }

    /// Detect language from file path (extension)
    pub fn detect(path: &Path) -> anyhow::Result<Self> {
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let lang = match extension.to_lowercase().as_str() {
            "rs" => Language::Rust,
            "ts" | "tsx" => Language::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "py" => Language::Python,
            "go" => Language::Go,
            "java" => Language::Java,
            "cs" => Language::CSharp,
            "rb" => Language::Ruby,
            "php" => Language::Php,
            "kt" | "kts" => Language::Kotlin,
            "swift" => Language::Swift,
            _ => Language::Unknown,
        };

        Ok(lang)
    }

    /// Get file extensions for this language
    pub fn extensions(&self) -> Vec<&'static str> {
        match self {
            Language::Rust => vec!["rs"],
            Language::TypeScript => vec!["ts", "tsx"],
            Language::JavaScript => vec!["js", "jsx", "mjs", "cjs"],
            Language::Python => vec!["py"],
            Language::Go => vec!["go"],
            Language::Java => vec!["java"],
            Language::CSharp => vec!["cs"],
            Language::Ruby => vec!["rb"],
            Language::Php => vec!["php"],
            Language::Kotlin => vec!["kt", "kts"],
            Language::Swift => vec!["swift"],
            Language::Unknown => vec![],
        }
    }
}

/// Whether a JS project is written in TypeScript.
///
/// A monorepo usually has no `tsconfig.json` at its root, only one per
/// package (`packages/*/tsconfig.json`), so the root alone is not enough.
/// The search is shallow and skips `node_modules` and dot-directories.
fn has_tsconfig(path: &Path) -> bool {
    walkdir::WalkDir::new(path)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_type().is_dir()
                || entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name != "node_modules" && !name.starts_with('.'))
        })
        .filter_map(|entry| entry.ok())
        .any(|entry| {
            entry.file_type().is_file()
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("tsconfig") && name.ends_with(".json"))
        })
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Language::Rust => write!(f, "Rust"),
            Language::TypeScript => write!(f, "TypeScript"),
            Language::JavaScript => write!(f, "JavaScript"),
            Language::Python => write!(f, "Python"),
            Language::Go => write!(f, "Go"),
            Language::Java => write!(f, "Java"),
            Language::CSharp => write!(f, "C#"),
            Language::Ruby => write!(f, "Ruby"),
            Language::Php => write!(f, "PHP"),
            Language::Kotlin => write!(f, "Kotlin"),
            Language::Swift => write!(f, "Swift"),
            Language::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Project entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub path: PathBuf,
    pub name: String,
    pub language: Language,
    pub fingerprint: Option<Fingerprint>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Code fingerprint - minimal structural snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fingerprint {
    pub project_id: Uuid,
    pub entry_points: Vec<PathBuf>,
    pub data_flows: Vec<DataFlow>,
    pub hot_zones: Vec<Zone>,
    pub cold_zones: Vec<Zone>,
    pub frozen_zones: Vec<Zone>,
    pub language: Language,
    pub generated_at: DateTime<Utc>,
    /// The project's code files, relative to its root.
    #[serde(default)]
    pub files: Vec<PathBuf>,
    /// Sub-projects found by their manifest (a workspace package, a crate…).
    #[serde(default)]
    pub modules: Vec<ModuleInfo>,
    /// The commit `HEAD` pointed at when the scan ran, if the project is a git
    /// repository. A different `HEAD` means the stored fingerprint is stale.
    #[serde(default)]
    pub head: Option<String>,
    /// Documentation files (Markdown), relative to the project root.
    #[serde(default)]
    pub docs: Vec<PathBuf>,
    /// Top-level declarations of every code file, and the headings (levels
    /// 1-3) of every document, for finding them by name.
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    /// Layout version of this fingerprint. A stored fingerprint with an older
    /// version lacks data newer tools rely on, so it is regenerated.
    #[serde(default)]
    pub format: u32,
}

/// A named declaration in the project (function, type, class…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    /// File relative to the project root.
    pub file: PathBuf,
    /// 1-based line of the declaration.
    pub line: usize,
}

/// A sub-project inside the scanned project, identified by its manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleInfo {
    /// Directory relative to the project root.
    pub path: PathBuf,
    /// Name from the manifest (`name` in package.json or Cargo.toml), or the
    /// directory name when the manifest has none.
    pub name: String,
    /// Manifest file name, e.g. `package.json`.
    pub manifest: String,
}

/// Data flow between modules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataFlow {
    pub from: PathBuf,
    /// The import as written in the source, e.g. `./util` or `@scope/pkg`.
    pub to: PathBuf,
    pub flow_type: FlowType,
    /// The project file the import points at, relative to the project root,
    /// when it could be resolved. `None` for external packages.
    #[serde(default)]
    pub resolved: Option<PathBuf>,
}

/// Type of data flow
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowType {
    Import,
    FunctionCall,
    Inheritance,
    Composition,
    Event,
}

/// Code zone with change frequency
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Zone {
    pub path: PathBuf,
    pub frequency: ZoneFrequency,
    pub last_modified: DateTime<Utc>,
    pub change_count: usize,
}

/// Zone frequency classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZoneFrequency {
    Hot,    // > 5 changes/month
    Warm,   // 2-5 changes/month
    Cold,   // < 2 changes/month
    Frozen, // no changes in 6 months
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monorepo_with_per_package_tsconfig_is_typescript() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("package.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join("packages/core")).unwrap();
        std::fs::write(root.join("packages/core/tsconfig.json"), "{}").unwrap();
        assert_eq!(
            Language::detect_project(root).unwrap(),
            Language::TypeScript
        );
    }

    #[test]
    fn test_tsconfig_inside_node_modules_does_not_count() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("package.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join("node_modules/lib")).unwrap();
        std::fs::write(root.join("node_modules/lib/tsconfig.json"), "{}").unwrap();
        assert_eq!(
            Language::detect_project(root).unwrap(),
            Language::JavaScript
        );
    }
}
