//! SQLite storage layer
//!
//! Provides SQLite persistence with modern features:
//! - WAL mode for concurrent reads
//! - mmap for fast reads
//! - FTS5 support ready for future features

use crate::types::Fingerprint;
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Schema version, bumped on every migration.
const SCHEMA_VERSION: i32 = 3;

/// Open a connection with all PRAGMAs applied.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    // Agents call tools in parallel, so several connections can open the same
    // database at once, and switching a new database to WAL takes a write
    // lock. SQLite answers that contention with SQLITE_BUSY without consulting
    // the busy handler, so the setup is serialized within this process and
    // retried across processes (a session hook scanning while the MCP server
    // opens the same database).
    static OPEN_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    let _guard = OPEN_LOCK.lock();

    let conn = Connection::open(path)
        .with_context(|| format!("Failed to open SQLite at {}", path.display()))?;
    let mut attempt = 0;
    loop {
        match apply_pragmas(&conn).and_then(|()| migrate(&conn)) {
            Ok(()) => return Ok(conn),
            Err(e) if attempt < 50 && is_busy(&e) => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(20 * attempt.min(10)));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Whether an error is SQLite reporting a locked database.
fn is_busy(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<rusqlite::Error>().is_some_and(|e| {
            matches!(
                e.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
            )
        })
    })
}

/// Apply performance and correctness PRAGMAs.
fn apply_pragmas(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA temp_store = MEMORY;
        PRAGMA mmap_size = 268435456;
        PRAGMA page_size = 8192;
        PRAGMA foreign_keys = ON;
        PRAGMA busy_timeout = 10000;
        PRAGMA optimize;
        ",
    )
    .context("Failed to apply PRAGMAs")?;
    Ok(())
}

/// Run migrations. Each version is idempotent.
fn migrate(conn: &Connection) -> Result<()> {
    let current: i32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);

    if current < 1 {
        migrate_v1(conn)?;
    }
    if current < 2 {
        migrate_v2(conn)?;
    }
    if current < 3 {
        migrate_v3(conn)?;
    }

    Ok(())
}

/// Public entry point for running every migration on a connection the caller
/// owns (in-memory tests, `Cogito::with_sqlite`).
pub fn migrate_pub(conn: &Connection) -> Result<()> {
    migrate(conn)
}

fn migrate_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        -- =====================
        -- Projects
        -- =====================
        CREATE TABLE IF NOT EXISTS projects (
            id          TEXT PRIMARY KEY,
            path        TEXT NOT NULL UNIQUE,
            name        TEXT NOT NULL,
            language    TEXT NOT NULL,
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );

        -- =====================
        -- Fingerprints cache
        -- =====================
        CREATE TABLE IF NOT EXISTS fingerprints (
            project_id           TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
            data                 BLOB NOT NULL,
            entry_points_count   INTEGER NOT NULL,
            data_flows_count     INTEGER NOT NULL,
            hot_zones_count      INTEGER NOT NULL,
            cold_zones_count     INTEGER NOT NULL,
            generated_at         TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_fingerprints_generated
            ON fingerprints(generated_at DESC);
        "#,
    )
    .context("Failed to run migration v1")?;

    // Record schema version
    conn.pragma_update(None, "user_version", 1i32)
        .context("Failed to set user_version")?;

    Ok(())
}

/// Migration v2: Remove legacy tables (decisions, tags) that are no longer used.
/// Forward migration: drop tables that may exist from a previous version.
fn migrate_v2(conn: &Connection) -> Result<()> {
    // The memory-era tables are gone from the product. Dropping them converges
    // an existing `state.db` instead of leaving dormant data behind. The FTS
    // tables are external-content tables over `decisions` and `tags`, so they
    // go first; each table's own triggers and indexes fall with it.
    conn.execute_batch(
        r#"
        DROP TABLE IF EXISTS decisions_fts;
        DROP TABLE IF EXISTS tags_fts;
        DROP TABLE IF EXISTS decisions;
        DROP TABLE IF EXISTS tags;
        "#,
    )
    .context("Failed to run migration v2")?;

    // Record schema version
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .context("Failed to set user_version")?;

    Ok(())
}

/// Migration v3: File and symbol index with FTS5 search.
///
/// - `indexed_files`: per-file metadata (size, mtime, content hash)
/// - `indexed_symbols`: declarations extracted from each file
/// - `indexed_symbols_fts`: FTS5 over symbols (name, path)
/// - `indexed_files_fts`: FTS5 over file paths
/// - `indexed_imports`: import specs and resolved paths
fn migrate_v3(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        -- =====================
        -- Indexed files
        -- =====================
        CREATE TABLE IF NOT EXISTS indexed_files (
            project_id  TEXT NOT NULL,
            path        TEXT NOT NULL,
            is_doc      INTEGER NOT NULL,
            language    TEXT NOT NULL,
            size        INTEGER NOT NULL,
            mtime_ns    INTEGER NOT NULL,
            content_hash INTEGER NOT NULL,
            indexed_at  TEXT NOT NULL,
            PRIMARY KEY (project_id, path)
        );

        CREATE INDEX IF NOT EXISTS idx_indexed_files_project
            ON indexed_files(project_id);

        -- =====================
        -- Indexed symbols
        -- =====================
        CREATE TABLE IF NOT EXISTS indexed_symbols (
            project_id  TEXT NOT NULL,
            path        TEXT NOT NULL,
            kind        TEXT NOT NULL,
            name        TEXT NOT NULL,
            start_line  INTEGER NOT NULL,
            end_line    INTEGER NOT NULL,
            signature   TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_indexed_symbols_project_path
            ON indexed_symbols(project_id, path);

        -- =====================
        -- FTS5: symbols (external-content)
        -- =====================
        CREATE VIRTUAL TABLE IF NOT EXISTS indexed_symbols_fts USING fts5(
            name,
            path,
            content='indexed_symbols',
            content_rowid='rowid',
            tokenize='unicode61'
        );

        CREATE TRIGGER IF NOT EXISTS indexed_symbols_ai AFTER INSERT ON indexed_symbols BEGIN
            INSERT INTO indexed_symbols_fts(rowid, name, path)
            VALUES (new.rowid, new.name, new.path);
        END;

        CREATE TRIGGER IF NOT EXISTS indexed_symbols_ad AFTER DELETE ON indexed_symbols BEGIN
            INSERT INTO indexed_symbols_fts(indexed_symbols_fts, rowid, name, path)
            VALUES ('delete', old.rowid, old.name, old.path);
        END;

        CREATE TRIGGER IF NOT EXISTS indexed_symbols_au AFTER UPDATE ON indexed_symbols BEGIN
            INSERT INTO indexed_symbols_fts(indexed_symbols_fts, rowid, name, path)
            VALUES ('delete', old.rowid, old.name, old.path);
            INSERT INTO indexed_symbols_fts(rowid, name, path)
            VALUES (new.rowid, new.name, new.path);
        END;

        -- =====================
        -- FTS5: files (external-content)
        -- =====================
        CREATE VIRTUAL TABLE IF NOT EXISTS indexed_files_fts USING fts5(
            path,
            content='indexed_files',
            content_rowid='rowid',
            tokenize='unicode61'
        );

        CREATE TRIGGER IF NOT EXISTS indexed_files_ai AFTER INSERT ON indexed_files BEGIN
            INSERT INTO indexed_files_fts(rowid, path)
            VALUES (new.rowid, new.path);
        END;

        CREATE TRIGGER IF NOT EXISTS indexed_files_ad AFTER DELETE ON indexed_files BEGIN
            INSERT INTO indexed_files_fts(indexed_files_fts, rowid, path)
            VALUES ('delete', old.rowid, old.path);
        END;

        CREATE TRIGGER IF NOT EXISTS indexed_files_au AFTER UPDATE ON indexed_files BEGIN
            INSERT INTO indexed_files_fts(indexed_files_fts, rowid, path)
            VALUES ('delete', old.rowid, old.path);
            INSERT INTO indexed_files_fts(rowid, path)
            VALUES (new.rowid, new.path);
        END;

        -- =====================
        -- Indexed imports
        -- =====================
        CREATE TABLE IF NOT EXISTS indexed_imports (
            project_id  TEXT NOT NULL,
            path        TEXT NOT NULL,
            spec        TEXT NOT NULL,
            resolved    TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_indexed_imports_project_resolved
            ON indexed_imports(project_id, resolved);
        "#,
    )
    .context("Failed to run migration v3")?;

    // Record schema version
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .context("Failed to set user_version")?;

    Ok(())
}

// ============================
// Project operations
// ============================

pub fn upsert_project(
    conn: &Connection,
    id: Uuid,
    path: &Path,
    name: &str,
    language: &str,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        r#"
        INSERT INTO projects (id, path, name, language, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?5)
        ON CONFLICT(id) DO UPDATE SET
            path = excluded.path,
            name = excluded.name,
            language = excluded.language,
            updated_at = excluded.updated_at
        "#,
        params![id.to_string(), path.to_string_lossy(), name, language, now],
    )?;
    Ok(())
}

// ============================
// Fingerprint operations
// ============================

/// Store `fp` as the project's current fingerprint, replacing any earlier one.
pub fn save_fingerprint(conn: &Connection, fp: &Fingerprint) -> Result<()> {
    let data = serde_json::to_vec(fp).context("Failed to serialize fingerprint")?;
    conn.execute(
        r#"
        INSERT INTO fingerprints (
            project_id, data, entry_points_count, data_flows_count,
            hot_zones_count, cold_zones_count, generated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(project_id) DO UPDATE SET
            data = excluded.data,
            entry_points_count = excluded.entry_points_count,
            data_flows_count = excluded.data_flows_count,
            hot_zones_count = excluded.hot_zones_count,
            cold_zones_count = excluded.cold_zones_count,
            generated_at = excluded.generated_at
        "#,
        params![
            fp.project_id.to_string(),
            data,
            fp.entry_points.len() as i64,
            fp.data_flows.len() as i64,
            fp.hot_zones.len() as i64,
            fp.cold_zones.len() as i64,
            fp.generated_at.to_rfc3339(),
        ],
    )
    .context("Failed to store fingerprint")?;
    Ok(())
}

/// The project's stored fingerprint, if it has been scanned.
pub fn load_fingerprint(conn: &Connection, project_id: Uuid) -> Result<Option<Fingerprint>> {
    let data: Option<Vec<u8>> = conn
        .query_row(
            "SELECT data FROM fingerprints WHERE project_id = ?1",
            params![project_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    data.map(|bytes| serde_json::from_slice(&bytes).context("Failed to read stored fingerprint"))
        .transpose()
}

// ============================
// Stats
// ============================

#[derive(Debug, Clone, serde::Serialize)]
pub struct Stats {
    pub project_id: Uuid,
    /// Whether a fingerprint has been generated for this project.
    pub has_fingerprint: bool,
    /// When the stored fingerprint was generated (RFC 3339), or `None` if the
    /// project has never been scanned.
    pub fingerprint_generated_at: Option<String>,
}

pub fn get_stats(conn: &Connection, project_id: Uuid) -> Result<Stats> {
    let pid_str = project_id.to_string();

    let fingerprint: Option<String> = conn
        .query_row(
            "SELECT generated_at FROM fingerprints WHERE project_id = ?1",
            params![pid_str],
            |row| row.get(0),
        )
        .optional()?;

    Ok(Stats {
        project_id,
        has_fingerprint: fingerprint.is_some(),
        fingerprint_generated_at: fingerprint,
    })
}

/// Path to the SQLite file for a given `.cogito/` directory.
pub fn db_path(cogito_dir: &Path) -> PathBuf {
    cogito_dir.join("state.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_db() -> (TempDir, Connection) {
        let dir = TempDir::new().unwrap();
        let conn = open(&db_path(dir.path())).unwrap();
        (dir, conn)
    }

    #[test]
    fn test_open_and_migrate() {
        let (_dir, conn) = test_db();
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn test_pragmas_applied() {
        let (_dir, conn) = test_db();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    }

    #[test]
    fn test_stats_no_fingerprint() {
        let (_dir, conn) = test_db();
        let project_id = Uuid::new_v4();
        upsert_project(
            &conn,
            project_id,
            std::path::Path::new("/test"),
            "test",
            "Rust",
        )
        .unwrap();

        let stats = get_stats(&conn, project_id).unwrap();
        assert!(!stats.has_fingerprint);
        assert!(stats.fingerprint_generated_at.is_none());
    }
}
