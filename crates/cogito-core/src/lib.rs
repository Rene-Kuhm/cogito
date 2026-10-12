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
pub mod semantic;
pub mod types;

use anyhow::Result;
use chrono::Utc;
use std::path::PathBuf;
use uuid::Uuid;

pub use types::*;

/// Main Cogito engine
#[derive(Debug, Clone)]
pub struct Cogito {
    #[allow(dead_code)] // No backend reads it yet: with_storage only stores it.
    storage: Option<StorageBackend>,
}

impl Cogito {
    /// Create a new Cogito instance
    pub fn new() -> Self {
        Self { storage: None }
    }

    /// Create with storage backend
    pub fn with_storage(storage: StorageBackend) -> Self {
        Self {
            storage: Some(storage),
        }
    }

    /// Initialize a new project
    pub fn init_project(&self, path: PathBuf) -> Result<Project> {
        let language = Language::detect(&path)?;

        Ok(Project {
            id: Uuid::new_v4(),
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

/// Storage backend trait
#[derive(Debug, Clone)]
pub enum StorageBackend {
    /// In-memory storage (for testing)
    Memory,
    /// SQLite storage
    Sqlite(PathBuf),
    /// PostgreSQL storage
    Postgres(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cogito_new() {
        let cogito = Cogito::new();
        assert!(cogito.storage.is_none());
    }

    #[test]
    fn test_language_detection() {
        let rust_path = PathBuf::from("/project/src/main.rs");
        assert_eq!(Language::detect(&rust_path).unwrap(), Language::Rust);
    }
}
