//! # Cogito Core
//!
//! Intelligent context harness for AI coding agents.
//!
//! ## Core Modules
//!
//! - [`fingerprint`] — Code fingerprint generator
//! - [`map`] — Project map and search
//! - [`storage`] — SQLite persistence
//!
//! ## Usage
//!
//! ```rust,no_run
//! use cogito_harness_core::Cogito;
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! let cogito = Cogito::new();
//! let project = cogito.init_project("/path/to/project".into())?;
//! let fingerprint = cogito.generate_fingerprint(&project).await?;
//! # Ok(())
//! # }
//! ```

pub mod fingerprint;
pub mod index;
pub mod map;
pub mod pack;
pub mod storage;
pub mod types;

use anyhow::Result;
use chrono::Utc;
use std::path::PathBuf;
use uuid::Uuid;

pub use types::*;

/// Keep `.cogito/` out of git in any repository: it holds a local index and
/// per-session state, never project content. A `.gitignore` of `*` inside the
/// directory ignores everything in it, itself included, without touching the
/// project's own `.gitignore`. Best effort: a failure only means `git status`
/// may list the directory.
fn ignore_in_git(cogito_dir: &std::path::Path) {
    let ignore = cogito_dir.join(".gitignore");
    if !ignore.exists() {
        let _ = std::fs::write(
            &ignore,
            "# Written by Cogito: this directory is a local index.\n*\n",
        );
    }
}

/// Main Cogito engine
///
/// A `Cogito` instance is bound to **one project's `.cogito` directory**.
/// Building a new instance for each project is the supported way to serve
/// several projects from one process (e.g. the MCP server): see
/// [`Cogito::for_project`].
#[derive(Debug, Clone)]
pub struct Cogito {
    /// SQLite connection, or `None` to use the legacy JSON store.
    sqlite: Option<std::sync::Arc<parking_lot::Mutex<rusqlite::Connection>>>,
    /// The project's `.cogito` directory. All persistence resolves against it.
    cogito_dir: PathBuf,
}

impl Cogito {
    /// Default `.cogito` directory next to the current working directory.
    ///
    /// Prefer [`Cogito::for_project`] so the location is explicit.
    pub fn new() -> Self {
        let cogito_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".cogito");
        Self {
            sqlite: None,
            cogito_dir,
        }
    }

    /// Open a Cogito instance bound to a project directory.
    ///
    /// Opens (creating if needed) `<project>/.cogito/state.db`. This is the
    /// correct constructor when the path is supplied by a caller — for
    /// example an MCP tool argument — rather than inherited from the process
    /// working directory.
    pub fn for_project(project: &std::path::Path) -> Result<Self> {
        Self::with_project_sqlite(&project.join(".cogito"))
    }

    /// Open a Cogito instance bound to a `.cogito` directory.
    pub fn with_project_sqlite(cogito_dir: &std::path::Path) -> Result<Self> {
        let db_path = crate::storage::db_path(cogito_dir);
        let conn = crate::storage::open(&db_path)?;
        ignore_in_git(cogito_dir);
        Ok(Self {
            sqlite: Some(std::sync::Arc::new(parking_lot::Mutex::new(conn))),
            cogito_dir: cogito_dir.to_path_buf(),
        })
    }

    /// Open a Cogito instance backed by an explicit SQLite file.
    pub fn with_sqlite(path: &std::path::Path) -> Result<Self> {
        let conn = crate::storage::open(path)?;
        let cogito_dir = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".cogito"));
        Ok(Self {
            sqlite: Some(std::sync::Arc::new(parking_lot::Mutex::new(conn))),
            cogito_dir,
        })
    }

    /// Open an in-memory SQLite Cogito (for tests).
    pub fn in_memory() -> Result<Self> {
        let conn = rusqlite::Connection::open_in_memory()?;
        crate::storage::sqlite::migrate_pub(&conn)?;
        Ok(Self {
            sqlite: Some(std::sync::Arc::new(parking_lot::Mutex::new(conn))),
            cogito_dir: PathBuf::from(".cogito"),
        })
    }

    /// The `.cogito` directory this instance is bound to.
    pub fn cogito_dir(&self) -> &std::path::Path {
        &self.cogito_dir
    }

    /// Get the SQLite connection if SQLite is enabled.
    /// Returns `None` if the engine is using the legacy JSON store.
    pub fn sqlite(&self) -> Option<&std::sync::Arc<parking_lot::Mutex<rusqlite::Connection>>> {
        self.sqlite.as_ref()
    }

    /// Borrow the connection, if any.
    pub fn connection(&self) -> Option<parking_lot::MutexGuard<'_, rusqlite::Connection>> {
        self.sqlite.as_ref().map(|c| c.lock())
    }

    /// Initialize a project rooted at `path`.
    pub fn init_project(&self, path: PathBuf) -> Result<Project> {
        let language = if path.is_dir() {
            Language::detect_project(&path)?
        } else {
            Language::detect(&path)?
        };

        // Deterministic id: identical canonical paths map to the same project.
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, canonical.to_string_lossy().as_bytes());

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

    /// Register a project row so foreign keys resolve.
    pub fn register_project(&self, project: &Project) -> Result<()> {
        if let Some(conn) = self.connection() {
            crate::storage::sqlite::upsert_project(
                &conn,
                project.id,
                &project.path,
                &project.name,
                &project.language.to_string(),
            )?;
        }
        Ok(())
    }

    /// Load the project rooted at `path`, registering it in SQLite.
    pub fn load_project(&self, path: PathBuf) -> Result<Project> {
        let project = self.init_project(path)?;
        self.register_project(&project)?;
        Ok(project)
    }

    /// Generate fingerprint for a project
    pub async fn generate_fingerprint(&self, project: &Project) -> Result<Fingerprint> {
        let fingerprint = fingerprint::generate(project).await?;
        // Store it, so stats and later sessions see the latest scan. A failed
        // write does not fail the scan: the fingerprint itself is still valid.
        if let Err(e) = self.store_fingerprint(project, &fingerprint) {
            tracing::warn!("[cogito] Could not store the fingerprint: {e:#}");
        }
        Ok(fingerprint)
    }

    /// Persist `fingerprint` as `project`'s current one.
    fn store_fingerprint(&self, project: &Project, fingerprint: &Fingerprint) -> Result<()> {
        // The fingerprint row references the project row.
        self.register_project(project)?;
        if let Some(conn) = self.connection() {
            crate::storage::sqlite::save_fingerprint(&conn, fingerprint)?;
        }
        Ok(())
    }

    /// The project's fingerprint, scanning only when needed.
    ///
    /// The stored fingerprint is reused unless `refresh` is set, the project
    /// was never scanned, the stored one predates the current format, or
    /// `HEAD` moved since it was taken. Returns whether a scan ran.
    ///
    /// Working-tree changes invalidate the stored fingerprint: the index is
    /// synced first and any change (added, modified, removed file) forces a
    /// regeneration so `fingerprint.files` and `fingerprint.symbols` always
    /// reflect the current working tree.
    ///
    /// The fast skip path (`allow_skip_when_clean = true`) is used only when
    /// a valid stored fingerprint exists and `HEAD` has not moved: the
    /// `worktree_changed()` check then avoids re-walking the tree.
    pub async fn fingerprint(
        &self,
        project: &Project,
        refresh: bool,
    ) -> Result<(Fingerprint, bool)> {
        // Load stored fingerprint before syncing so we know whether to allow
        // the fast skip path.
        let stored = if self.sqlite.is_some() {
            self.stored_fingerprint(project.id)?
        } else {
            None
        };

        let can_skip = !refresh
            && stored.is_some()
            && stored
                .as_ref()
                .map(|s| s.format >= fingerprint::FINGERPRINT_FORMAT)
                .unwrap_or(false)
            && stored
                .as_ref()
                .map(|s| s.head == fingerprint::current_head(&project.path))
                .unwrap_or(false);

        let index_changed = if let Some(conn) = self.connection() {
            let report = crate::index::sync(&conn, project.id, &project.path, can_skip)?;
            report.changed()
        } else {
            false
        };

        if !refresh
            && !index_changed
            && stored
                .as_ref()
                .map(|s| {
                    s.format >= fingerprint::FINGERPRINT_FORMAT
                        && s.head == fingerprint::current_head(&project.path)
                })
                .unwrap_or(false)
        {
            return Ok((stored.unwrap(), false));
        }
        Ok((self.generate_fingerprint(project).await?, true))
    }

    /// Sync the index for `project`, returning a report.
    ///
    /// This is the cheap freshness entry point: it checks whether the working
    /// tree has changed and re-indexes only what is necessary, but it does NOT
    /// regenerate the fingerprint. Use this before any query that needs the
    /// index up to date. `Cogito::fingerprint` is for `project_map` and for
    /// explicit scan/refresh paths.
    pub async fn sync_index(&self, project: &Project) -> Result<crate::index::SyncReport> {
        // Load stored fingerprint to determine the allow_skip_when_clean flag.
        let stored = self.stored_fingerprint(project.id)?;
        let can_skip = stored
            .as_ref()
            .map(|s| {
                s.format >= fingerprint::FINGERPRINT_FORMAT
                    && s.head == fingerprint::current_head(&project.path)
            })
            .unwrap_or(false);

        let conn = self
            .connection()
            .ok_or_else(|| anyhow::anyhow!("No SQLite connection"))?;
        crate::index::sync(&conn, project.id, &project.path, can_skip)
    }

    /// The fingerprint stored by the last scan, if any.
    pub fn stored_fingerprint(&self, project_id: Uuid) -> Result<Option<Fingerprint>> {
        match self.connection() {
            Some(conn) => crate::storage::sqlite::load_fingerprint(&conn, project_id),
            None => Ok(None),
        }
    }

    /// Storage statistics for a project
    pub fn stats(&self, project_id: Uuid) -> Result<crate::storage::Stats> {
        match self.connection() {
            Some(conn) => crate::storage::sqlite::get_stats(&conn, project_id),
            None => Ok(crate::storage::Stats {
                project_id,
                has_fingerprint: false,
                fingerprint_generated_at: None,
            }),
        }
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
