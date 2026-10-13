//! File and symbol index with incremental sync and FTS5 search.
//!
//! The index is a SQLite-backed store of every project file's metadata,
//! declarations, and import graph. It stays in sync with the working tree
//! by comparing (size, mtime) pairs, so files whose bytes have not changed
//! (even with a newer mtime) are never re-parsed.

use crate::fingerprint::project_file_paths;
use crate::map::layout::Resolver;
use crate::map::outline::{declarations, is_doc};
use crate::map::symbols::Declaration;
use crate::types::Language;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Re-export CODE_EXTENSIONS for use by external consumers of the index API.
pub use crate::fingerprint::CODE_EXTENSIONS;

// ---------------------------------------------------------------------------
// Report types
// ---------------------------------------------------------------------------

/// What a sync pass did.
#[derive(Debug, Clone, Default)]
pub struct SyncReport {
    pub scanned: usize,
    pub added: usize,
    pub modified: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub reparsed: usize,
}

impl SyncReport {
    /// Whether anything changed.
    pub fn changed(&self) -> bool {
        self.added > 0 || self.modified > 0 || self.removed > 0
    }
}

/// One search result.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub kind: &'static str,
    pub name: String,
    pub path: PathBuf,
    pub line: usize,
    pub score: f64,
}

/// One declaration indexed from a file.
#[derive(Debug, Clone)]
pub struct IndexedDeclaration {
    pub kind: String,
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub signature: Option<String>,
}

impl From<Declaration> for IndexedDeclaration {
    fn from(d: Declaration) -> Self {
        Self {
            kind: d.kind,
            name: d.name,
            start: d.start,
            end: d.end,
            signature: d.signature,
        }
    }
}

// ---------------------------------------------------------------------------
// FNV-1a hash (no external dependency)
// ---------------------------------------------------------------------------

/// 64-bit FNV-1a hash of a byte slice.
fn fnv1a(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

// ---------------------------------------------------------------------------
// Metadata helpers
// ---------------------------------------------------------------------------

fn file_mtime_ns(path: &Path) -> std::io::Result<u64> {
    let meta = fs::metadata(path)?;
    let sys_time = meta.modified()?;
    let duration = sys_time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| SystemTime::now().duration_since(UNIX_EPOCH).unwrap());
    Ok(duration.as_nanos() as u64)
}

// ---------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------

/// Scan the project tree and sync the index.
///
/// Files whose (size, mtime_ns) match the stored row are skipped entirely.
/// If only size/mtime differ but the content hash matches, only the metadata
/// is updated. Otherwise the file is re-parsed and its declarations and
/// imports are re-indexed.
///
/// Files present in the index but absent from the file set are removed.
pub fn sync(conn: &Connection, project_id: Uuid, root: &Path) -> Result<SyncReport> {
    let mut report = SyncReport::default();

    // Collect the current file set, excluding the .cogito directory
    let current: HashSet<PathBuf> = project_file_paths(root)
        .filter(|p| !p.starts_with(".cogito"))
        .collect();
    report.scanned = current.len();

    // Collect already-indexed paths
    let mut indexed: HashSet<PathBuf> = HashSet::new();
    let mut indexed_stmt = conn.prepare("SELECT path FROM indexed_files WHERE project_id = ?1")?;
    let rows = indexed_stmt.query_map(params![project_id.to_string()], |row| {
        Ok(PathBuf::from(row.get::<_, String>(0)?))
    })?;
    for row in rows {
        indexed.insert(row?);
    }
    drop(indexed_stmt);

    // Remove vanished files
    for vanished in indexed.difference(&current) {
        let path_str = vanished.to_string_lossy();
        // FTS entries are deleted by the DELETE trigger on indexed_symbols
        conn.execute(
            "DELETE FROM indexed_symbols WHERE project_id = ?1 AND path = ?2",
            params![project_id.to_string(), path_str],
        )?;
        conn.execute(
            "DELETE FROM indexed_imports WHERE project_id = ?1 AND path = ?2",
            params![project_id.to_string(), path_str],
        )?;
        // Delete file metadata (file FTS entry becomes orphaned but that's acceptable)
        conn.execute(
            "DELETE FROM indexed_files WHERE project_id = ?1 AND path = ?2",
            params![project_id.to_string(), path_str],
        )?;
        report.removed += 1;
    }

    // Process current files
    let root_abs = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    for rel_path in &current {
        let abs = root_abs.join(rel_path);
        let ext = abs.extension().and_then(|e| e.to_str()).unwrap_or_default();

        // Get stored metadata, if any
        let stored: Option<(i64, u64, u64)> = conn
            .query_row(
                "SELECT size, mtime_ns, content_hash FROM indexed_files WHERE project_id = ?1 AND path = ?2",
                params![project_id.to_string(), rel_path.to_string_lossy()],
                |row| Ok((row.get(0)?, row.get::<_, i64>(1)? as u64, row.get::<_, i64>(2)? as u64)),
            )
            .optional()?;

        let Ok(meta) = fs::metadata(&abs) else {
            continue;
        };
        let size = meta.len() as i64;
        let mtime_ns = file_mtime_ns(&abs)?;

        match stored {
            Some((s, m, _hash)) if s == size && m == mtime_ns => {
                // Unchanged: skip content read entirely
                report.unchanged += 1;
                continue;
            }
            Some((_, _, hash)) => {
                // Metadata differs: read and hash to confirm
                let Ok(content) = fs::read(&abs) else {
                    continue;
                };
                let content_hash = fnv1a(&content);

                if content_hash == hash {
                    // Same content, just update metadata
                    let now = chrono::Utc::now().to_rfc3339();
                    conn.execute(
                        "UPDATE indexed_files SET size = ?1, mtime_ns = ?2, indexed_at = ?3 WHERE project_id = ?4 AND path = ?5",
                        params![size, mtime_ns as i64, now, project_id.to_string(), rel_path.to_string_lossy()],
                    )?;
                    report.unchanged += 1;
                    continue;
                }

                // Content actually changed: reparse
                reparse_file(conn, project_id, &abs, rel_path, ext, &content)?;
                report.modified += 1;
                report.reparsed += 1;
            }
            None => {
                // New file
                let Ok(content) = fs::read(&abs) else {
                    continue;
                };
                reparse_file(conn, project_id, &abs, rel_path, ext, &content)?;
                report.added += 1;
                report.reparsed += 1;
            }
        }
    }

    Ok(report)
}

/// Index (or re-index) a single file.
fn reparse_file(
    conn: &Connection,
    project_id: Uuid,
    abs: &Path,
    rel_path: &Path,
    ext: &str,
    content: &[u8],
) -> Result<()> {
    let content_hash = fnv1a(content);
    let size = content.len() as i64;
    let mtime_ns = file_mtime_ns(abs)?;
    let now = chrono::Utc::now().to_rfc3339();
    let path_str = rel_path.to_string_lossy().to_string();
    let is_doc = is_doc(ext) as i32;
    let language = ext.to_lowercase();

    // Upsert file row
    conn.execute(
        r#"INSERT INTO indexed_files (project_id, path, is_doc, language, size, mtime_ns, content_hash, indexed_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
           ON CONFLICT(project_id, path) DO UPDATE SET
               is_doc = excluded.is_doc,
               language = excluded.language,
               size = excluded.size,
               mtime_ns = excluded.mtime_ns,
               content_hash = excluded.content_hash,
               indexed_at = excluded.indexed_at"#,
        params![
            project_id.to_string(),
            path_str,
            is_doc,
            language,
            size,
            mtime_ns as i64,
            content_hash as i64,
            now
        ],
    )?;

    // Delete old symbols (FTS rows are deleted by trigger)
    conn.execute(
        "DELETE FROM indexed_symbols WHERE project_id = ?1 AND path = ?2",
        params![project_id.to_string(), path_str],
    )?;

    // Parse declarations
    let content_str = String::from_utf8_lossy(content);
    let decls: Vec<Declaration> = declarations(&content_str, ext);
    for decl in decls {
        conn.execute(
            r#"INSERT INTO indexed_symbols (project_id, path, kind, name, start_line, end_line, signature)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                project_id.to_string(),
                path_str,
                decl.kind,
                decl.name,
                decl.start as i64,
                decl.end as i64,
                decl.signature,
            ],
        )?;
    }

    // Extract and resolve imports
    conn.execute(
        "DELETE FROM indexed_imports WHERE project_id = ?1 AND path = ?2",
        params![project_id.to_string(), path_str],
    )?;

    if let Ok(lang) = Language::detect(abs) {
        let root = abs
            .parent()
            .and_then(|p| p.ancestors().nth(1))
            .unwrap_or_else(|| Path::new("."));
        let files: Vec<PathBuf> = Vec::new();
        let resolver = Resolver::new(root, &files, &[]);

        let raw_imports = crate::fingerprint::extract_imports(abs, &content_str, &lang)?;
        for flow in raw_imports {
            let resolved = resolver
                .resolve(rel_path, &flow.to.to_string_lossy(), lang)
                .map(|p| p.to_string_lossy().to_string());
            conn.execute(
                r#"INSERT INTO indexed_imports (project_id, path, spec, resolved)
                   VALUES (?1, ?2, ?3, ?4)"#,
                params![
                    project_id.to_string(),
                    path_str,
                    flow.to.to_string_lossy(),
                    resolved,
                ],
            )?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// Search symbols and files by name.
pub fn search(
    conn: &Connection,
    project_id: Uuid,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    let mut hits = Vec::new();

    // Symbol search via FTS5
    let fts_query = format!("\"{query}\"*");
    if let Ok(mut stmt) = conn.prepare(
        r#"SELECT s.name, s.path, s.start_line, s.kind
           FROM indexed_symbols s
           JOIN indexed_symbols_fts f ON s.rowid = f.rowid
           WHERE indexed_symbols_fts MATCH ?1 AND s.project_id = ?2
           ORDER BY rank
           LIMIT ?3"#,
    ) {
        let rows = stmt.query_map(
            params![fts_query, project_id.to_string(), limit as i64],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        );
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                hits.push(SearchHit {
                    kind: "symbol",
                    name: row.0,
                    path: PathBuf::from(&row.1),
                    line: row.2 as usize,
                    score: 1.0,
                });
            }
        }
    }

    // File search via file FTS5
    if let Ok(mut stmt) = conn.prepare(
        r#"SELECT f.path, f.is_doc
           FROM indexed_files f
           JOIN indexed_files_fts ft ON f.rowid = ft.rowid
           WHERE indexed_files_fts MATCH ?1 AND f.project_id = ?2
           ORDER BY rank
           LIMIT ?3"#,
    ) {
        let rows = stmt.query_map(
            params![fts_query, project_id.to_string(), (limit / 2) as i64],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?)),
        );
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                hits.push(SearchHit {
                    kind: if row.1 == 1 { "doc" } else { "file" },
                    name: Path::new(&row.0)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    path: PathBuf::from(&row.0),
                    line: 0,
                    score: 0.5,
                });
            }
        }
    }

    // Sort by score descending, then trim
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(limit);
    Ok(hits)
}

// ---------------------------------------------------------------------------
// Outline
// ---------------------------------------------------------------------------

/// All declarations indexed from a file.
pub fn outline(
    conn: &Connection,
    project_id: Uuid,
    path: &Path,
) -> Result<Vec<IndexedDeclaration>> {
    let path_str = path.to_string_lossy();
    let mut stmt = conn.prepare(
        r#"SELECT kind, name, start_line, end_line, signature
           FROM indexed_symbols
           WHERE project_id = ?1 AND path = ?2
           ORDER BY start_line"#,
    )?;
    let rows = stmt.query_map(params![project_id.to_string(), path_str], |row| {
        Ok(IndexedDeclaration {
            kind: row.get(0)?,
            name: row.get(1)?,
            start: row.get::<_, i64>(2)? as usize,
            end: row.get::<_, i64>(3)? as usize,
            signature: row.get(4)?,
        })
    })?;
    let result: Result<Vec<_>, _> = rows.collect();
    Ok(result?)
}

// ---------------------------------------------------------------------------
// Imports
// ---------------------------------------------------------------------------

/// All import specs in a file.
pub fn imports(conn: &Connection, project_id: Uuid, path: &Path) -> Result<Vec<String>> {
    let path_str = path.to_string_lossy();
    let mut stmt =
        conn.prepare(r#"SELECT spec FROM indexed_imports WHERE project_id = ?1 AND path = ?2"#)?;
    let rows = stmt.query_map(params![project_id.to_string(), path_str], |row| row.get(0))?;
    let result: Result<Vec<_>, _> = rows.collect();
    Ok(result?)
}

/// All files that import the given path (by resolved path or spec).
pub fn importers(conn: &Connection, project_id: Uuid, path: &Path) -> Result<Vec<PathBuf>> {
    let path_str = path.to_string_lossy();
    let mut stmt = conn.prepare(
        r#"SELECT DISTINCT path FROM indexed_imports
           WHERE project_id = ?1 AND (resolved = ?2 OR spec = ?2)"#,
    )?;
    let rows = stmt.query_map(params![project_id.to_string(), path_str], |row| {
        Ok(PathBuf::from(row.get::<_, String>(0)?))
    })?;
    let result: Result<Vec<_>, _> = rows.collect();
    Ok(result?)
}

// ---------------------------------------------------------------------------
// Project removal
// ---------------------------------------------------------------------------

/// Delete all index data for a project.
pub fn remove_project(conn: &Connection, project_id: Uuid) -> Result<()> {
    // FTS entries are deleted by the DELETE trigger on indexed_symbols
    conn.execute(
        "DELETE FROM indexed_files WHERE project_id = ?1",
        params![project_id.to_string()],
    )?;
    conn.execute(
        "DELETE FROM indexed_symbols WHERE project_id = ?1",
        params![project_id.to_string()],
    )?;
    conn.execute(
        "DELETE FROM indexed_imports WHERE project_id = ?1",
        params![project_id.to_string()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Fast path: check git worktree for modifications
// ---------------------------------------------------------------------------

/// Whether the git worktree has modified, added or deleted files.
pub fn worktree_changed(root: &Path) -> bool {
    let repo = match git2::Repository::discover(root) {
        Ok(r) => r,
        Err(_) => return true, // Treat discovery failure as "changed"
    };
    let _workdir = match repo.workdir() {
        Some(w) => w,
        None => return true,
    };

    // Use diff with workdir to detect any changes
    let diff = match repo.diff_index_to_workdir(None, None) {
        Ok(d) => d,
        Err(_) => return true,
    };

    if diff.deltas().len() > 0 {
        return true;
    }

    // Check for untracked files in the working tree
    let mut status_opts = git2::StatusOptions::new();
    status_opts.include_untracked(true);
    let statuses = match repo.statuses(Some(&mut status_opts)) {
        Ok(s) => s,
        Err(_) => return true,
    };

    for entry in statuses.iter() {
        let status = entry.status();
        if status.is_index_new()
            || status.is_index_modified()
            || status.is_index_deleted()
            || status.is_wt_new()
            || status.is_wt_modified()
            || status.is_wt_deleted()
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fnv1a_known() {
        // Known test vector for FNV-1a
        assert_eq!(fnv1a(b"hello"), 0xa430d84680aabd0b);
    }

    #[test]
    fn test_sync_report_changed() {
        let empty = SyncReport::default();
        assert!(!empty.changed());

        let with_added = SyncReport {
            added: 1,
            ..Default::default()
        };
        assert!(with_added.changed());
    }
}
