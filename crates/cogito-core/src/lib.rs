//! # Cogito Core
//!
//! Intelligent context optimization engine for AI coding agents.
//!
//! ## Core Modules
//!
//! - [`fingerprint`] — Code Fingerprint generator
//! - [`compressor`] — Context compression engine
//! - [`semantic`] — Semantic Memory Tags
//! - [`decision`] — Decision Log engine
//!
//! ## Usage
//!
//! ```rust,no_run
//! use cogito_core::Cogito;
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! let cogito = Cogito::new();
//! let project = cogito.init_project("/path/to/project".into())?;
//! let fingerprint = cogito.generate_fingerprint(&project).await?;
//! # Ok(())
//! # }
//! ```

pub mod compressor;
pub mod decision;
pub mod fingerprint;
pub mod persistence;
pub mod semantic;
pub mod storage;
pub mod types;

use anyhow::Result;
use chrono::Utc;
use std::path::PathBuf;
use uuid::Uuid;

pub use types::*;

/// Main Cogito engine
#[derive(Debug, Clone)]
pub struct Cogito {
    /// Optional SQLite connection (None = use legacy JSON file storage)
    sqlite: Option<std::sync::Arc<parking_lot::Mutex<rusqlite::Connection>>>,
}

impl Cogito {
    /// Create a new Cogito instance
    pub fn new() -> Self {
        Self { sqlite: None }
    }

    /// Open a Cogito instance backed by SQLite at the given path
    /// (e.g. `.cogito/state.db`). Idempotent and safe to call repeatedly.
    pub fn with_sqlite(path: &std::path::Path) -> Result<Self> {
        let conn = crate::storage::open(path)?;
        let arc = std::sync::Arc::new(parking_lot::Mutex::new(conn));
        crate::semantic::set_sqlite(arc.clone());
        crate::decision::set_sqlite(arc.clone());
        Ok(Self { sqlite: Some(arc) })
    }

    /// Open a Cogito instance backed by the project's `.cogito/state.db`.
    /// Auto-migration happens later in `init_project` once the project_id is known.
    pub fn with_project_sqlite(cogito_dir: &std::path::Path) -> Result<Self> {
        let db_path = crate::storage::db_path(cogito_dir);
        let conn = crate::storage::open(&db_path)?;
        let arc = std::sync::Arc::new(parking_lot::Mutex::new(conn));
        crate::semantic::set_sqlite(arc.clone());
        crate::decision::set_sqlite(arc.clone());
        Ok(Self { sqlite: Some(arc) })
    }

    /// Open an in-memory SQLite Cogito (for tests)
    pub fn in_memory() -> Result<Self> {
        let conn = rusqlite::Connection::open_in_memory()?;
        crate::storage::sqlite::migrate_v1_pub(&conn)?;
        let arc = std::sync::Arc::new(parking_lot::Mutex::new(conn));
        crate::semantic::set_sqlite(arc.clone());
        crate::decision::set_sqlite(arc.clone());
        Ok(Self { sqlite: Some(arc) })
    }

    /// Get the SQLite connection if SQLite is enabled.
    /// Returns None if the engine is using legacy JSON storage.
    pub fn sqlite(&self) -> Option<&std::sync::Arc<parking_lot::Mutex<rusqlite::Connection>>> {
        self.sqlite.as_ref()
    }

    /// Initialize a new project
    pub fn init_project(&self, path: PathBuf) -> Result<Project> {
        // Detect language: if path is a directory, use project detection; otherwise use file extension
        let language = if path.is_dir() {
            Language::detect_project(&path)?
        } else {
            Language::detect(&path)?
        };

        // Use a deterministic project_id based on the absolute path so
        // decisions/tags persist across CLI invocations.
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, canonical.to_string_lossy().as_bytes());

        // If SQLite is open, run auto-migration from JSON to SQLite,
        // re-assigning migrated records to this project_id.
        if let Some(sqlite) = self.sqlite() {
            let cogito_dir = path.join(".cogito");
            if cogito_dir.exists() {
                let conn = sqlite.lock();
                if let Ok(report) =
                    crate::storage::sqlite::migrate_from_json(&conn, &cogito_dir, Some(id))
                    && (report.tags_imported > 0 || report.decisions_imported > 0) {
                        tracing::info!(
                            "Migrated {} tags and {} decisions from legacy JSON",
                            report.tags_imported,
                            report.decisions_imported
                        );
                    }
            }
        }

        Ok(Project {
            id,
            path: path.clone(),
            name: path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string(),
            language,
            fingerprint: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    /// Generate fingerprint for a project
    pub async fn generate_fingerprint(&self, project: &Project) -> Result<Fingerprint> {
        fingerprint::generate(project).await
    }

    /// Compress context
    pub async fn compress_context(
        &self,
        content: &str,
        options: CompressionOptions,
    ) -> Result<CompressedContext> {
        compressor::compress(content, options)
    }

    /// Search semantic tags
    pub async fn search_tags(&self, project_id: Uuid, query: &str) -> Result<Vec<SemanticTag>> {
        semantic::search(project_id, query).await
    }

    /// Add semantic tag
    pub async fn add_tag(&self, tag: SemanticTag) -> Result<SemanticTag> {
        semantic::add(tag).await
    }

    /// Create decision
    pub async fn add_decision(&self, decision: Decision) -> Result<Decision> {
        decision::add(decision).await
    }

    /// Search decisions
    pub async fn search_decisions(&self, project_id: Uuid, query: &str) -> Result<Vec<Decision>> {
        decision::search(project_id, query).await
    }

    /// Get hot/cold zones
    pub async fn get_zones(&self, project: &Project) -> Result<Vec<Zone>> {
        fingerprint::detect_zones(project).await
    }
}

impl Default for Cogito {
    fn default() -> Self {
        Self::new()
    }
}

// StorageBackend enum removed: SQLite is now the default,
// legacy JSON is implicit. See `Cogito::with_sqlite` and `with_project_sqlite`.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cogito_new() {
        let cogito = Cogito::new();
        assert!(cogito.sqlite.is_none());
    }

    #[test]
    fn test_language_detection() {
        let rust_path = PathBuf::from("/project/src/main.rs");
        assert_eq!(Language::detect(&rust_path).unwrap(), Language::Rust);
    }
}
