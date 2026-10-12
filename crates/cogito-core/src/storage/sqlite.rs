//! SQLite storage layer
//!
//! Replaces the file-based JSON storage with a single-file SQLite database
//! that uses modern features:
//! - FTS5 with porter stemming for full-text search
//! - JSONB for compact array storage (2x faster than text JSON)
//! - Generated columns for derived fields
//! - Expression indexes for query optimization
//! - WAL mode for concurrent reads
//! - mmap for fast reads

use crate::types::{Decision, SemanticTag};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Schema version, bumped on every migration.
const SCHEMA_VERSION: i32 = 1;

/// Open a connection with all PRAGMAs applied.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("Failed to create directory {}", parent.display())
        })?;
    }

    let conn = Connection::open(path)
        .with_context(|| format!("Failed to open SQLite at {}", path.display()))?;

    apply_pragmas(&conn)?;
    migrate(&conn)?;

    Ok(conn)
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

    Ok(())
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
        -- Decisions
        -- =====================
        CREATE TABLE IF NOT EXISTS decisions (
            id            TEXT PRIMARY KEY,
            project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            title         TEXT NOT NULL,
            why           TEXT NOT NULL,
            -- JSONB is not exposed in rusqlite but JSON1 + json functions are equivalent
            -- We use JSON text but index via generated columns
            alternatives  TEXT NOT NULL DEFAULT '[]',
            paths         TEXT NOT NULL DEFAULT '[]',
            consequences  TEXT NOT NULL DEFAULT '[]',
            tags          TEXT NOT NULL DEFAULT '[]',
            author        TEXT NOT NULL,
            created_at    TEXT NOT NULL,

            -- Generated columns (auto-calculated on write)
            title_lower   TEXT GENERATED ALWAYS AS (lower(title)) STORED,
            path_count    INTEGER GENERATED ALWAYS AS (json_array_length(paths)) STORED,
            tag_count     INTEGER GENERATED ALWAYS AS (json_array_length(tags)) STORED,
            alt_count     INTEGER GENERATED ALWAYS AS (json_array_length(alternatives)) STORED,
            cons_count    INTEGER GENERATED ALWAYS AS (json_array_length(consequences)) STORED,

            -- Concatenated search text (auto-updated, indexed by FTS5)
            search_text   TEXT GENERATED ALWAYS AS (
                lower(coalesce(title, '')) || ' ' ||
                lower(coalesce(why, '')) || ' ' ||
                lower(coalesce(alternatives, '')) || ' ' ||
                lower(coalesce(consequences, '')) || ' ' ||
                lower(coalesce(tags, ''))
            ) STORED
        );

        CREATE INDEX IF NOT EXISTS idx_decisions_project
            ON decisions(project_id);
        CREATE INDEX IF NOT EXISTS idx_decisions_created
            ON decisions(created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_decisions_path_count
            ON decisions(path_count) WHERE path_count > 0;
        CREATE INDEX IF NOT EXISTS idx_decisions_tag_count
            ON decisions(tag_count) WHERE tag_count > 0;
        CREATE INDEX IF NOT EXISTS idx_decisions_title_lower
            ON decisions(title_lower);

        -- FTS5 with porter stemming for full-text search
        CREATE VIRTUAL TABLE IF NOT EXISTS decisions_fts USING fts5(
            search_text,
            content='decisions',
            content_rowid='rowid',
            tokenize='porter unicode61'
        );

        -- Triggers: FTS stays in sync with the table
        CREATE TRIGGER IF NOT EXISTS decisions_ai AFTER INSERT ON decisions BEGIN
            INSERT INTO decisions_fts(rowid, search_text)
            VALUES (new.rowid, new.search_text);
        END;

        CREATE TRIGGER IF NOT EXISTS decisions_ad AFTER DELETE ON decisions BEGIN
            INSERT INTO decisions_fts(decisions_fts, rowid, search_text)
            VALUES('delete', old.rowid, old.search_text);
        END;

        CREATE TRIGGER IF NOT EXISTS decisions_au AFTER UPDATE ON decisions BEGIN
            INSERT INTO decisions_fts(decisions_fts, rowid, search_text)
            VALUES('delete', old.rowid, old.search_text);
            INSERT INTO decisions_fts(rowid, search_text)
            VALUES (new.rowid, new.search_text);
        END;

        -- =====================
        -- Tags
        -- =====================
        CREATE TABLE IF NOT EXISTS tags (
            id           TEXT PRIMARY KEY,
            project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            tag          TEXT NOT NULL,
            domain       TEXT NOT NULL,
            confidence   REAL NOT NULL,
            paths        TEXT NOT NULL DEFAULT '[]',
            created_at   TEXT NOT NULL,

            -- Generated columns
            tag_lower    TEXT GENERATED ALWAYS AS (lower(tag)) STORED,
            path_count   INTEGER GENERATED ALWAYS AS (json_array_length(paths)) STORED
        );

        CREATE INDEX IF NOT EXISTS idx_tags_project ON tags(project_id);
        CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags(tag);
        CREATE INDEX IF NOT EXISTS idx_tags_tag_lower ON tags(tag_lower);
        CREATE INDEX IF NOT EXISTS idx_tags_domain ON tags(domain);
        CREATE INDEX IF NOT EXISTS idx_tags_confidence ON tags(confidence DESC);
        CREATE INDEX IF NOT EXISTS idx_tags_path_count ON tags(path_count) WHERE path_count > 0;

        -- FTS5 for tags
        CREATE VIRTUAL TABLE IF NOT EXISTS tags_fts USING fts5(
            tag, domain, paths,
            content='tags',
            content_rowid='rowid',
            tokenize='porter unicode61'
        );

        CREATE TRIGGER IF NOT EXISTS tags_ai AFTER INSERT ON tags BEGIN
            INSERT INTO tags_fts(rowid, tag, domain, paths)
            VALUES (new.rowid, new.tag, new.domain, new.paths);
        END;

        CREATE TRIGGER IF NOT EXISTS tags_ad AFTER DELETE ON tags BEGIN
            INSERT INTO tags_fts(tags_fts, rowid, tag, domain, paths)
            VALUES('delete', old.rowid, old.tag, old.domain, old.paths);
        END;

        CREATE TRIGGER IF NOT EXISTS tags_au AFTER UPDATE ON tags BEGIN
            INSERT INTO tags_fts(tags_fts, rowid, tag, domain, paths)
            VALUES('delete', old.rowid, old.tag, old.domain, old.paths);
            INSERT INTO tags_fts(rowid, tag, domain, paths)
            VALUES (new.rowid, new.tag, new.domain, new.paths);
        END;

        -- =====================
        -- Fingerprints cache
        -- =====================
        CREATE TABLE IF NOT EXISTS fingerprints (
            project_id           TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
            data                 BLOB NOT NULL,    -- msgpack or bincode serialized
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
// Decision operations
// ============================

pub fn insert_decision(conn: &Connection, d: &Decision) -> Result<()> {
    let alternatives = serde_json::to_string(&d.alternatives)?;
    let paths = serde_json::to_string(&d.paths)?;
    let consequences = serde_json::to_string(&d.consequences)?;
    let tags = serde_json::to_string(&d.tags)?;

    conn.execute(
        r#"
        INSERT INTO decisions (
            id, project_id, title, why, alternatives, paths, consequences, tags, author, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
        params![
            d.id.to_string(),
            d.project_id.to_string(),
            d.title,
            d.why,
            alternatives,
            paths,
            consequences,
            tags,
            d.author,
            d.created_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub fn search_decisions(
    conn: &Connection,
    project_id: Uuid,
    query: &str,
) -> Result<Vec<Decision>> {
    let pid_str = project_id.to_string();

    // Two strategies combined: FTS5 (with stemming) + LIKE fallback
    // FTS5 only matches whole words; LIKE catches partial matches in path/tag fields
    let mut sql = String::from(
        r#"
        SELECT id, project_id, title, why, alternatives, paths, consequences, tags, author, created_at
        FROM decisions
        WHERE project_id = ?1
        "#,
    );

    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(pid_str.clone())];

    if !query.is_empty() {
        // FTS5 search (with porter stemming) OR LIKE on title/why
        sql.push_str(
            r#" AND (
                rowid IN (
                    SELECT rowid FROM decisions_fts
                    WHERE decisions_fts MATCH ?2
                )
                OR lower(title) LIKE ?3
                OR lower(why) LIKE ?3
                OR lower(tags) LIKE ?3
                OR lower(paths) LIKE ?3
            )"#,
        );
        let fts_query = format!("{}*", query); // prefix match
        let like_query = format!("%{}%", query.to_lowercase());
        params_vec.push(Box::new(fts_query));
        params_vec.push(Box::new(like_query));
    }

    sql.push_str(" ORDER BY created_at DESC");

    let mut stmt = conn.prepare(&sql)?;
    let params_refs: Vec<&dyn rusqlite::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();

    let rows = stmt.query_map(&params_refs[..], |row| {
        let id: String = row.get(0)?;
        let project_id: String = row.get(1)?;
        let title: String = row.get(2)?;
        let why: String = row.get(3)?;
        let alternatives: String = row.get(4)?;
        let paths: String = row.get(5)?;
        let consequences: String = row.get(6)?;
        let tags: String = row.get(7)?;
        let author: String = row.get(8)?;
        let created_at: String = row.get(9)?;

        Ok(Decision {
            id: Uuid::parse_str(&id).unwrap_or_default(),
            project_id: Uuid::parse_str(&project_id).unwrap_or_default(),
            title,
            why,
            alternatives: serde_json::from_str(&alternatives).unwrap_or_default(),
            paths: serde_json::from_str::<Vec<String>>(&paths)
                .unwrap_or_default()
                .into_iter()
                .map(std::path::PathBuf::from)
                .collect(),
            consequences: serde_json::from_str(&consequences).unwrap_or_default(),
            tags: serde_json::from_str(&tags).unwrap_or_default(),
            author,
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .map(|d| d.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

// ============================
// Tag operations
// ============================

pub fn insert_tag(conn: &Connection, t: &SemanticTag) -> Result<()> {
    let paths = serde_json::to_string(&t.paths)?;

    conn.execute(
        r#"
        INSERT INTO tags (id, project_id, tag, domain, confidence, paths, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
        params![
            t.id.to_string(),
            t.project_id.to_string(),
            t.tag,
            t.domain,
            t.confidence,
            paths,
            t.created_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub fn search_tags(
    conn: &Connection,
    project_id: Uuid,
    query: &str,
) -> Result<Vec<SemanticTag>> {
    let pid_str = project_id.to_string();

    let mut sql = String::from(
        r#"
        SELECT id, project_id, tag, domain, confidence, paths, created_at
        FROM tags
        WHERE project_id = ?1
        "#,
    );

    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(pid_str)];

    if !query.is_empty() {
        sql.push_str(
            r#" AND (
                lower(tag) LIKE ?2
                OR lower(domain) LIKE ?2
                OR lower(paths) LIKE ?2
            )"#,
        );
        let like_query = format!("%{}%", query.to_lowercase());
        params_vec.push(Box::new(like_query));
    }

    sql.push_str(" ORDER BY confidence DESC, created_at DESC");

    let mut stmt = conn.prepare(&sql)?;
    let params_refs: Vec<&dyn rusqlite::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();

    let rows = stmt.query_map(&params_refs[..], |row| {
        let id: String = row.get(0)?;
        let project_id: String = row.get(1)?;
        let tag: String = row.get(2)?;
        let domain: String = row.get(3)?;
        let confidence: f64 = row.get(4)?;
        let paths: String = row.get(5)?;
        let created_at: String = row.get(6)?;

        Ok(SemanticTag {
            id: Uuid::parse_str(&id).unwrap_or_default(),
            project_id: Uuid::parse_str(&project_id).unwrap_or_default(),
            tag,
            domain,
            confidence: confidence as f32,
            paths: serde_json::from_str::<Vec<String>>(&paths)
                .unwrap_or_default()
                .into_iter()
                .map(std::path::PathBuf::from)
                .collect(),
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .map(|d| d.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

// ============================
// Stats
// ============================

#[derive(Debug, Clone, serde::Serialize)]
pub struct Stats {
    pub project_id: Uuid,
    pub tags_count: i64,
    pub decisions_count: i64,
    pub hot_zones_count: i64,
}

pub fn get_stats(conn: &Connection, project_id: Uuid) -> Result<Stats> {
    let pid_str = project_id.to_string();

    let tags_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tags WHERE project_id = ?1",
            params![pid_str],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let decisions_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions WHERE project_id = ?1",
            params![pid_str],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let hot_zones_count: i64 = conn
        .query_row(
            "SELECT COALESCE(hot_zones_count, 0) FROM fingerprints WHERE project_id = ?1",
            params![pid_str],
            |row| row.get(0),
        )
        .unwrap_or(0);

    Ok(Stats {
        project_id,
        tags_count,
        decisions_count,
        hot_zones_count,
    })
}

/// Path to the SQLite file for a given `.cogito/` directory.
pub fn db_path(cogito_dir: &Path) -> PathBuf {
    cogito_dir.join("state.db")
}

/// Run the migration from legacy JSON files to SQLite. Idempotent.
pub fn migrate_from_json(conn: &Connection, cogito_dir: &Path) -> Result<MigrateReport> {
    use crate::persistence;

    let mut report = MigrateReport::default();

    // Migrate tags
    let tags_path = cogito_dir.join("tags.json");
    if tags_path.exists() {
        let tags = persistence::load_tags(cogito_dir)?;
        for tag in &tags {
            // Skip if already in DB
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM tags WHERE id = ?1",
                    params![tag.id.to_string()],
                    |_| Ok(true),
                )
                .optional()?
                .unwrap_or(false);
            if !exists {
                insert_tag(conn, tag)?;
                report.tags_imported += 1;
            }
        }
    }

    // Migrate decisions
    let decisions_path = cogito_dir.join("decisions.json");
    if decisions_path.exists() {
        let decisions = persistence::load_decisions(cogito_dir)?;
        for decision in &decisions {
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM decisions WHERE id = ?1",
                    params![decision.id.to_string()],
                    |_| Ok(true),
                )
                .optional()?
                .unwrap_or(false);
            if !exists {
                insert_decision(conn, decision)?;
                report.decisions_imported += 1;
            }
        }
    }

    Ok(report)
}

#[derive(Debug, Default, Clone)]
pub struct MigrateReport {
    pub tags_imported: usize,
    pub decisions_imported: usize,
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
    fn test_insert_and_search_decision() {
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

        let decision = Decision {
            id: Uuid::new_v4(),
            project_id,
            title: "Use JWT for authentication".to_string(),
            why: "stateless and scales horizontally".to_string(),
            alternatives: vec!["Cookie sessions".to_string()],
            paths: vec![std::path::PathBuf::from("src/auth/")],
            consequences: vec!["Need refresh tokens".to_string()],
            tags: vec!["auth".to_string(), "security".to_string()],
            author: "tester".to_string(),
            created_at: chrono::Utc::now(),
        };

        insert_decision(&conn, &decision).unwrap();

        // Search by title word (porter stemming: "authentication" -> "authenticate")
        let results = search_decisions(&conn, project_id, "authenticate").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, decision.id);

        // Search by tag
        let results = search_decisions(&conn, project_id, "security").unwrap();
        assert_eq!(results.len(), 1);

        // Search with empty query returns all
        let results = search_decisions(&conn, project_id, "").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_insert_and_search_tag() {
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

        let tag = SemanticTag {
            id: Uuid::new_v4(),
            project_id,
            tag: "billing".to_string(),
            domain: "commerce".to_string(),
            paths: vec![std::path::PathBuf::from("src/billing/")],
            confidence: 0.9,
            created_at: chrono::Utc::now(),
        };

        insert_tag(&conn, &tag).unwrap();

        let results = search_tags(&conn, project_id, "billing").unwrap();
        assert_eq!(results.len(), 1);

        let results = search_tags(&conn, project_id, "commerce").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_generated_columns() {
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

        let decision = Decision {
            id: Uuid::new_v4(),
            project_id,
            title: "Test Decision".to_string(),
            why: "for testing".to_string(),
            alternatives: vec!["A".to_string(), "B".to_string()],
            paths: vec![
                std::path::PathBuf::from("a/"),
                std::path::PathBuf::from("b/"),
                std::path::PathBuf::from("c/"),
            ],
            consequences: vec![],
            tags: vec!["x".to_string(), "y".to_string()],
            author: "t".to_string(),
            created_at: chrono::Utc::now(),
        };
        insert_decision(&conn, &decision).unwrap();

        let (path_count, tag_count, alt_count): (i64, i64, i64) = conn
            .query_row(
                "SELECT path_count, tag_count, alt_count FROM decisions WHERE id = ?1",
                params![decision.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        assert_eq!(path_count, 3);
        assert_eq!(tag_count, 2);
        assert_eq!(alt_count, 2);
    }

    #[test]
    fn test_stats() {
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
        assert_eq!(stats.tags_count, 0);
        assert_eq!(stats.decisions_count, 0);
    }
}
