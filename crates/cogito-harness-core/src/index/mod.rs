//! File and symbol index with incremental sync and FTS5 search.
//!
//! The index is a SQLite-backed store of every project file's metadata,
//! declarations, and import graph. It stays in sync with the working tree
//! by comparing (size, mtime) pairs, so files whose bytes have not changed
//! (even with a newer mtime) are never re-parsed.

use crate::fingerprint::project_file_paths;
use crate::map::layout::{Resolver, find_modules};
use crate::map::outline::{declarations, is_doc};
use crate::map::symbols::Declaration;
use crate::types::Language;
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::{BTreeSet, HashSet};
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
    /// The symbol kind for symbol hits (e.g., `"function"`, `"class"`), or
    /// the file type for file/doc hits (`"file"` or `"doc"`).
    pub kind: String,
    /// Whether this is a symbol hit (vs a file/doc hit).
    pub is_symbol: bool,
    /// The matched symbol name or file name.
    pub name: String,
    /// Path to the file containing the match.
    pub path: PathBuf,
    /// 1-based line number for symbol hits; `0` for file/doc hits.
    pub line: usize,
    /// Ranking signal. In the single-phase FTS5 path this is the bm25 rank
    /// (**lower = more relevant**). In the two-phase bounded path this is the
    /// match-tier score: `3.0` = exact name match, `2.0` = name prefix match,
    /// `1.0` = name contains match, `0.0` = other. Do not reverse the sign.
    pub score: f64,
    /// Whether this hit lives inside a test directory or test file.
    pub is_test: bool,
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

impl From<IndexedDeclaration> for crate::map::outline::Declaration {
    fn from(d: IndexedDeclaration) -> Self {
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
// Test-path classification
// ---------------------------------------------------------------------------

/// Return true if `path` is inside a test directory or has a test-like name.
///
/// A test path is one where any path component matches (case-insensitively)
/// `tests`, `test`, `spec`, `specs`, `__tests__`, or where the file name
/// contains `.test.`, `_test.`, `.spec.`, `_spec.`, or starts with `test_`.
fn is_test_path(path: &Path) -> bool {
    let components: Vec<String> = path
        .components()
        .filter_map(|c| c.as_os_str().to_string_lossy().into_owned().into())
        .collect();

    // Check directory components
    for comp in &components[..components.len().saturating_sub(1)] {
        let lower = comp.to_lowercase();
        if lower == "tests"
            || lower == "test"
            || lower == "spec"
            || lower == "specs"
            || lower == "__tests__"
        {
            return true;
        }
    }

    // Check the file name (last component)
    if let Some(filename) = components.last() {
        let lower = filename.to_lowercase();
        lower.contains(".test.")
            || lower.contains("_test.")
            || lower.contains(".spec.")
            || lower.contains("_spec.")
            || lower.starts_with("test_")
    } else {
        false
    }
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
///
/// When `allow_skip_when_clean` is true and the git worktree is clean,
/// returns immediately with an unchanged report without walking the tree.
pub fn sync(
    conn: &Connection,
    project_id: Uuid,
    root: &Path,
    allow_skip_when_clean: bool,
) -> Result<SyncReport> {
    // Fast path: skip the walk entirely when the worktree is clean.
    if allow_skip_when_clean && !worktree_changed(root) {
        let scanned = conn
            .query_row(
                "SELECT COUNT(*) FROM indexed_files WHERE project_id = ?1",
                params![project_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize;

        return Ok(SyncReport {
            scanned,
            unchanged: scanned,
            ..Default::default()
        });
    }

    let mut report = SyncReport::default();

    // Collect the current file set, excluding the .cogito directory
    let current: HashSet<PathBuf> = project_file_paths(root)
        .filter(|p| !p.starts_with(".cogito"))
        .collect();
    report.scanned = current.len();

    // Find workspace modules for import resolution
    let files_vec: Vec<PathBuf> = current.iter().cloned().collect();
    let modules = find_modules(root, &files_vec);

    // Collect already-indexed paths
    let mut indexed: HashSet<PathBuf> = HashSet::new();
    let mut indexed_stmt =
        conn.prepare_cached("SELECT path FROM indexed_files WHERE project_id = ?1")?;
    let rows = indexed_stmt.query_map(params![project_id.to_string()], |row| {
        Ok(PathBuf::from(row.get::<_, String>(0)?))
    })?;
    for row in rows {
        indexed.insert(row?);
    }
    drop(indexed_stmt);

    // Batch size for committing transactions. Smaller batches reduce commit time
    // but more batches mean more transaction overhead. 5000 is a good balance for
    // large projects.
    const BATCH_SIZE: usize = 5000;

    // Remove vanished files in one transaction
    {
        let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
        let mut delete_symbols =
            tx.prepare_cached("DELETE FROM indexed_symbols WHERE project_id = ?1 AND path = ?2")?;
        let mut delete_imports =
            tx.prepare_cached("DELETE FROM indexed_imports WHERE project_id = ?1 AND path = ?2")?;
        let mut delete_files =
            tx.prepare_cached("DELETE FROM indexed_files WHERE project_id = ?1 AND path = ?2")?;

        let pid = project_id.to_string();
        for vanished in indexed.difference(&current) {
            let path_str = vanished.to_string_lossy();
            delete_symbols.execute(params![&pid, path_str.as_ref()])?;
            delete_imports.execute(params![&pid, path_str.as_ref()])?;
            delete_files.execute(params![&pid, path_str.as_ref()])?;
            report.removed += 1;
        }
        drop(delete_symbols);
        drop(delete_imports);
        drop(delete_files);
        tx.commit()?;
    }

    // Process current files in batches
    let pid = project_id.to_string();
    let root_abs = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let files_refs: Vec<&PathBuf> = current.iter().collect();

    // Build the resolver once, before the batch loop.
    // Resolver::new is O(n) in file count; building it per-file made sync O(n²).
    let resolver = Resolver::new(&root_abs, &files_vec, &modules);

    for batch in files_refs.chunks(BATCH_SIZE) {
        // Each batch is its own transaction for commit efficiency
        let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;

        // Prepared statements for this batch
        let mut select_metadata = tx.prepare_cached(
            "SELECT size, mtime_ns, content_hash FROM indexed_files WHERE project_id = ?1 AND path = ?2",
        )?;
        let mut update_metadata = tx.prepare_cached(
            "UPDATE indexed_files SET size = ?1, mtime_ns = ?2, indexed_at = ?3 WHERE project_id = ?4 AND path = ?5",
        )?;
        let mut reparse_stmts = ReparseStmts::new(&tx)?;

        for rel_path in batch {
            let abs = root_abs.join(rel_path);
            let ext = abs.extension().and_then(|e| e.to_str()).unwrap_or_default();
            let rel_path_str = rel_path.to_string_lossy();

            // Get stored metadata, if any
            let stored: Option<(i64, u64, u64)> = select_metadata
                .query_row(params![&pid, rel_path_str.as_ref()], |row| {
                    Ok((
                        row.get(0)?,
                        row.get::<_, i64>(1)? as u64,
                        row.get::<_, i64>(2)? as u64,
                    ))
                })
                .optional()?;

            let Ok(meta) = fs::metadata(&abs) else {
                continue;
            };
            let size = meta.len() as i64;
            let mtime_ns = file_mtime_ns(&abs)?;

            match stored {
                Some((s, m, _hash)) if s == size && m == mtime_ns => {
                    report.unchanged += 1;
                    continue;
                }
                Some((_, _, hash)) => {
                    let Ok(content) = fs::read(&abs) else {
                        continue;
                    };
                    let content_hash = fnv1a(&content);

                    if content_hash == hash {
                        let now = chrono::Utc::now().to_rfc3339();
                        update_metadata.execute(params![
                            size,
                            mtime_ns as i64,
                            &now,
                            &pid,
                            rel_path_str.as_ref()
                        ])?;
                        report.unchanged += 1;
                        continue;
                    }

                    reparse_file(
                        project_id,
                        &abs,
                        rel_path,
                        ext,
                        &content,
                        &resolver,
                        &mut reparse_stmts,
                    )?;
                    report.modified += 1;
                    report.reparsed += 1;
                }
                None => {
                    let Ok(content) = fs::read(&abs) else {
                        continue;
                    };
                    reparse_file(
                        project_id,
                        &abs,
                        rel_path,
                        ext,
                        &content,
                        &resolver,
                        &mut reparse_stmts,
                    )?;
                    report.added += 1;
                    report.reparsed += 1;
                }
            }
        }

        drop(select_metadata);
        drop(update_metadata);
        drop(reparse_stmts);
        tx.commit()?;
    }

    Ok(report)
}

/// Index (or re-index) a single file.
///
/// Called within a transaction that wraps the entire sync run, so all work for
/// all files is committed atomically in one transaction. Uses pre-prepared
/// statements for efficiency.
fn reparse_file(
    project_id: Uuid,
    abs: &Path,
    rel_path: &Path,
    ext: &str,
    content: &[u8],
    resolver: &Resolver,
    stmts: &mut ReparseStmts,
) -> Result<()> {
    let content_hash = fnv1a(content);
    let size = content.len() as i64;
    let mtime_ns = file_mtime_ns(abs)?;
    let now = chrono::Utc::now().to_rfc3339();
    let path_str = rel_path.to_string_lossy().to_string();
    let is_doc = is_doc(ext) as i32;
    let language = ext.to_lowercase();
    let pid = project_id.to_string();

    // Upsert file row
    stmts.upsert_file.execute(params![
        &pid,
        &path_str,
        is_doc,
        &language,
        size,
        mtime_ns as i64,
        content_hash as i64,
        &now
    ])?;

    // Delete old symbols (FTS rows are deleted by trigger)
    stmts.delete_symbols.execute(params![&pid, &path_str])?;

    // Parse declarations
    let content_str = String::from_utf8_lossy(content);
    let decls: Vec<Declaration> = declarations(&content_str, ext);
    for decl in decls {
        stmts.insert_symbol.execute(params![
            &pid,
            &path_str,
            &decl.kind,
            &decl.name,
            decl.start as i64,
            decl.end as i64,
            &decl.signature,
        ])?;
    }

    // Extract and resolve imports
    stmts.delete_imports.execute(params![&pid, &path_str])?;

    if let Ok(lang) = Language::detect(abs) {
        let raw_imports = crate::fingerprint::extract_imports(abs, &content_str, &lang)?;
        for flow in raw_imports {
            let resolved = resolver
                .resolve(rel_path, &flow.to.to_string_lossy(), lang)
                .map(|p| p.to_string_lossy().to_string());
            stmts.insert_import.execute(params![
                &pid,
                &path_str,
                &flow.to.to_string_lossy(),
                resolved,
            ])?;
        }
    }

    Ok(())
}

/// Prepared statements for reparse_file, created once and reused for all files.
struct ReparseStmts<'tx> {
    upsert_file: rusqlite::CachedStatement<'tx>,
    delete_symbols: rusqlite::CachedStatement<'tx>,
    insert_symbol: rusqlite::CachedStatement<'tx>,
    delete_imports: rusqlite::CachedStatement<'tx>,
    insert_import: rusqlite::CachedStatement<'tx>,
}

impl<'tx> ReparseStmts<'tx> {
    fn new(tx: &'tx Transaction) -> Result<Self> {
        let upsert_file = tx
            .prepare_cached(
                r#"INSERT INTO indexed_files (project_id, path, is_doc, language, size, mtime_ns, content_hash, indexed_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                   ON CONFLICT(project_id, path) DO UPDATE SET
                       is_doc = excluded.is_doc,
                       language = excluded.language,
                       size = excluded.size,
                       mtime_ns = excluded.mtime_ns,
                       content_hash = excluded.content_hash,
                       indexed_at = excluded.indexed_at"#,
            )
            .context("Failed to prepare upsert_file statement")?;
        let delete_symbols = tx
            .prepare_cached("DELETE FROM indexed_symbols WHERE project_id = ?1 AND path = ?2")
            .context("Failed to prepare delete_symbols statement")?;
        let insert_symbol = tx
            .prepare_cached(
                r#"INSERT INTO indexed_symbols (project_id, path, kind, name, start_line, end_line, signature)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            )
            .context("Failed to prepare insert_symbol statement")?;
        let delete_imports = tx
            .prepare_cached("DELETE FROM indexed_imports WHERE project_id = ?1 AND path = ?2")
            .context("Failed to prepare delete_imports statement")?;
        let insert_import = tx
            .prepare_cached(
                r#"INSERT INTO indexed_imports (project_id, path, spec, resolved)
                   VALUES (?1, ?2, ?3, ?4)"#,
            )
            .context("Failed to prepare insert_import statement")?;
        Ok(Self {
            upsert_file,
            delete_symbols,
            insert_symbol,
            delete_imports,
            insert_import,
        })
    }
}

// ---------------------------------------------------------------------------
// Search helpers
// ---------------------------------------------------------------------------

/// Build a safe FTS5 MATCH expression from a user query.
///
/// Splits on whitespace, quotes each token (escaping embedded quotes),
/// appends a prefix `*`, then joins with ` AND `.
fn build_fts_query(query: &str) -> String {
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| {
            let escaped = t.replace('"', "\"\"");
            format!("\"{escaped}\"*")
        })
        .collect();
    tokens.join(" AND ")
}

/// Compute a match-tier score for a hit given the query tokens.
///
/// Higher tier = better match:
/// - 3.0: case-insensitive exact name match
/// - 2.0: name starts with query (prefix match)
/// - 1.0: name contains query (substring match)
/// - 0.0: token appears in FTS5 but not in name (FTS5 matched path or doc)
fn name_match_tier(name: &str, query: &str) -> f64 {
    let lower_name = name.to_lowercase();
    let query_lower = query.to_lowercase();
    if lower_name == query_lower {
        3.0
    } else if lower_name.starts_with(&query_lower) {
        2.0
    } else if lower_name.contains(&query_lower) {
        1.0
    } else {
        0.0
    }
}

/// Escape SQL LIKE meta-characters so they are matched literally.
fn like_escape(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Run an FTS5 search using a two-phase bounded approach.
///
/// Phase 1 (no ORDER BY): collect up to `candidates` rowids per class cheaply,
/// so FTS5 terminates early on common tokens.
///
/// Phase 2: fetch full rows by rowid and rank in Rust by match-tier,
/// non-test before test, then path, then line.
fn fts_search(
    conn: &Connection,
    project_id: Uuid,
    fts_query: &str,
    query: &str,
    candidates: usize,
) -> Result<Vec<SearchHit>> {
    if fts_query.is_empty() {
        return Ok(Vec::new());
    }

    let mut hits = Vec::new();
    let sym_candidates = candidates;
    let file_candidates = candidates / 2;

    // ── Phase 1: symbol rowids (no ORDER BY) ─────────────────────────────────
    let sym_rowids: Vec<i64> = match conn.prepare(
        "SELECT rowid FROM indexed_symbols_fts WHERE indexed_symbols_fts MATCH ?1 LIMIT ?2",
    ) {
        Ok(mut stmt) => stmt
            .query_map(params![fts_query, sym_candidates as i64], |row| row.get(0))
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect(),
        Err(_) => Vec::new(),
    };

    // ── Phase 2: fetch symbol rows by rowid, rank in Rust ───────────────────
    if !sym_rowids.is_empty() {
        let placeholders: String = sym_rowids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT name, path, start_line, kind FROM indexed_symbols \
             WHERE project_id = ?1 AND rowid IN ({placeholders})"
        );
        let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(project_id.to_string())];
        for &id in &sym_rowids {
            all_params.push(Box::new(id));
        }
        let param_refs: Vec<&dyn rusqlite::ToSql> = all_params.iter().map(|b| b.as_ref()).collect();

        if let Ok(mut stmt) = conn.prepare(&sql) {
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            });
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    let path = PathBuf::from(&row.1);
                    hits.push(SearchHit {
                        kind: row.3.clone(),
                        is_symbol: true,
                        name: row.0.clone(),
                        path: path.clone(),
                        line: row.2 as usize,
                        score: name_match_tier(&row.0, query),
                        is_test: is_test_path(&path),
                    });
                }
            }
        }
    }

    // ── Phase 1: file rowids (no ORDER BY) ───────────────────────────────────
    let file_rowids: Vec<i64> = match conn
        .prepare("SELECT rowid FROM indexed_files_fts WHERE indexed_files_fts MATCH ?1 LIMIT ?2")
    {
        Ok(mut stmt) => stmt
            .query_map(params![fts_query, file_candidates as i64], |row| row.get(0))
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect(),
        Err(_) => Vec::new(),
    };

    // ── Phase 2: fetch file rows by rowid, rank in Rust ─────────────────────
    if !file_rowids.is_empty() {
        let placeholders: String = file_rowids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT path, is_doc FROM indexed_files \
             WHERE project_id = ?1 AND rowid IN ({placeholders})"
        );
        let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(project_id.to_string())];
        for &id in &file_rowids {
            all_params.push(Box::new(id));
        }
        let param_refs: Vec<&dyn rusqlite::ToSql> = all_params.iter().map(|b| b.as_ref()).collect();

        if let Ok(mut stmt) = conn.prepare(&sql) {
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?))
            });
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    let path = PathBuf::from(&row.0);
                    let filename = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    hits.push(SearchHit {
                        kind: if row.1 == 1 { "doc" } else { "file" }.to_string(),
                        is_symbol: false,
                        name: filename.clone(),
                        path: path.clone(),
                        line: 0,
                        score: name_match_tier(&filename, query),
                        is_test: is_test_path(&path),
                    });
                }
            }
        }
    }

    Ok(hits)
}

/// LIKE scan for when the FTS5 expression is invalid or returns nothing.
///
/// LIKE scans use bounded candidates and rank in Rust, matching the two-phase
/// approach for consistency.
fn like_scan(
    conn: &Connection,
    project_id: Uuid,
    query: &str,
    candidates: usize,
) -> Result<Vec<SearchHit>> {
    let escaped = like_escape(query);
    let pattern = format!("%{escaped}%");
    let mut hits = Vec::new();

    // Symbol LIKE scan
    let sym_rowids: Vec<i64> = match conn.prepare(
        "SELECT rowid FROM indexed_symbols \
         WHERE project_id = ?1 AND name LIKE ?2 LIMIT ?3",
    ) {
        Ok(mut stmt) => stmt
            .query_map(
                params![project_id.to_string(), &pattern, candidates as i64],
                |row| row.get(0),
            )
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect(),
        Err(_) => Vec::new(),
    };

    if !sym_rowids.is_empty() {
        let placeholders: String = sym_rowids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT name, path, start_line, kind FROM indexed_symbols \
             WHERE project_id = ?1 AND rowid IN ({placeholders})"
        );
        let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(project_id.to_string())];
        for &id in &sym_rowids {
            all_params.push(Box::new(id));
        }
        let param_refs: Vec<&dyn rusqlite::ToSql> = all_params.iter().map(|b| b.as_ref()).collect();

        if let Ok(mut stmt) = conn.prepare(&sql) {
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            });
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    let path = PathBuf::from(&row.1);
                    hits.push(SearchHit {
                        kind: row.3.clone(),
                        is_symbol: true,
                        name: row.0.clone(),
                        path: path.clone(),
                        line: row.2 as usize,
                        score: name_match_tier(&row.0, query),
                        is_test: is_test_path(&path),
                    });
                }
            }
        }
    }

    // File LIKE scan
    let file_rowids: Vec<i64> = match conn.prepare(
        "SELECT rowid FROM indexed_files \
         WHERE project_id = ?1 AND path LIKE ?2 LIMIT ?3",
    ) {
        Ok(mut stmt) => stmt
            .query_map(
                params![project_id.to_string(), &pattern, (candidates / 2) as i64],
                |row| row.get(0),
            )
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect(),
        Err(_) => Vec::new(),
    };

    if !file_rowids.is_empty() {
        let placeholders: String = file_rowids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT path, is_doc FROM indexed_files \
             WHERE project_id = ?1 AND rowid IN ({placeholders})"
        );
        let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(project_id.to_string())];
        for &id in &file_rowids {
            all_params.push(Box::new(id));
        }
        let param_refs: Vec<&dyn rusqlite::ToSql> = all_params.iter().map(|b| b.as_ref()).collect();

        if let Ok(mut stmt) = conn.prepare(&sql) {
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?))
            });
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    let path = PathBuf::from(&row.0);
                    let filename = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    hits.push(SearchHit {
                        kind: if row.1 == 1 { "doc" } else { "file" }.to_string(),
                        is_symbol: false,
                        name: filename.clone(),
                        path: path.clone(),
                        line: 0,
                        score: name_match_tier(&filename, query),
                        is_test: is_test_path(&path),
                    });
                }
            }
        }
    }

    Ok(hits)
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// Search symbols and files by name.
///
/// Tokens are split on whitespace, each token is quoted and prefixed with `*`,
/// then joined with ` AND ` to form a safe FTS5 MATCH expression. If the
/// expression produces an FTS5 error (e.g. a lone `*` or `"`) a LIKE scan over
/// `indexed_symbols.name` and `indexed_files.path` is used instead.
///
/// Phase 1 collects candidate rowids without ORDER BY so FTS5 terminates early
/// on common tokens. Phase 2 fetches those rows and ranks them in Rust.
///
/// Results are sorted: symbols before files; within each class, non-test paths
/// before test paths; then by match-tier (exact > prefix > contains > other);
/// then by path; then by line number.
pub fn search(
    conn: &Connection,
    project_id: Uuid,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    let fts_query = build_fts_query(query);
    let candidates = (limit * 20).clamp(200, 2000);

    // Try two-phase FTS5 search first
    let mut hits = fts_search(conn, project_id, &fts_query, query, candidates)?;

    // If FTS returned nothing and the query had meaningful content, fall back to LIKE
    if hits.is_empty() && !fts_query.trim().is_empty() && !query.trim().is_empty() {
        hits = like_scan(conn, project_id, query, candidates)?;
    }

    // Sort: symbols before files, within each class non-test before test,
    // then match-tier (higher = better), then path, then line.
    hits.sort_by(|a, b| {
        // Class: symbol = 0, file/doc = 1
        let a_class = if a.is_symbol { 0 } else { 1 };
        let b_class = if b.is_symbol { 0 } else { 1 };
        if a_class != b_class {
            return a_class.cmp(&b_class);
        }
        // Within class: non-test before test
        if a.is_test != b.is_test {
            return a.is_test.cmp(&b.is_test); // false < true
        }
        // Within class+test: match-tier (higher = better)
        if (a.score - b.score).abs() > 1e-9 {
            return b
                .score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal);
        }
        // Tie-break: path
        let path_cmp = a.path.cmp(&b.path);
        if path_cmp != std::cmp::Ordering::Equal {
            return path_cmp;
        }
        // Tie-break: line
        a.line.cmp(&b.line)
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
/// Results are deduplicated and deterministically ordered.
/// Prefers resolved paths over raw specs when available.
pub fn imports(conn: &Connection, project_id: Uuid, path: &Path) -> Result<Vec<String>> {
    let path_str = path.to_string_lossy();
    let mut stmt = conn.prepare(
        r#"SELECT COALESCE(resolved, spec) FROM indexed_imports WHERE project_id = ?1 AND path = ?2"#,
    )?;
    let rows = stmt.query_map(params![project_id.to_string(), path_str], |row| row.get(0))?;
    let mut set: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        set.insert(row?);
    }
    Ok(set.into_iter().collect())
}

/// All files that import the given path (by resolved path or spec).
/// Results are deduplicated and deterministically ordered.
pub fn importers(conn: &Connection, project_id: Uuid, path: &Path) -> Result<Vec<PathBuf>> {
    let path_str = path.to_string_lossy();
    let mut stmt = conn.prepare(
        r#"SELECT DISTINCT path FROM indexed_imports
           WHERE project_id = ?1 AND (resolved = ?2 OR spec = ?2)"#,
    )?;
    let rows = stmt.query_map(params![project_id.to_string(), path_str], |row| {
        Ok(PathBuf::from(row.get::<_, String>(0)?))
    })?;
    let mut set: BTreeSet<PathBuf> = BTreeSet::new();
    for row in rows {
        set.insert(row?);
    }
    Ok(set.into_iter().collect())
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    #[test]
    fn test_is_test_path() {
        // Test directory components
        assert!(is_test_path(Path::new("apps/web/tests/auth.test.ts")));
        assert!(is_test_path(Path::new(
            "packages/utils/test/helpers.spec.ts"
        )));
        assert!(is_test_path(Path::new(
            "src/__tests__/integration.spec.tsx"
        )));
        assert!(is_test_path(Path::new("spec/unit.spec.js")));

        // Test file name patterns
        assert!(is_test_path(Path::new("lib/math.test.ts")));
        assert!(is_test_path(Path::new("lib/math_spec.ts")));
        assert!(is_test_path(Path::new("helpers_test.py")));
        assert!(is_test_path(Path::new("helpers.spec.js")));

        // Not test paths
        assert!(!is_test_path(Path::new("apps/web/lib/auth/server.ts")));
        assert!(!is_test_path(Path::new("src/main.rs")));
        assert!(!is_test_path(Path::new("packages/core/src/lib.rs")));
        assert!(!is_test_path(Path::new("docs/guide.md")));
        assert!(!is_test_path(Path::new("README.md")));

        // Edge: no components
        assert!(!is_test_path(Path::new("")));
    }

    #[test]
    fn test_build_fts_query() {
        assert_eq!(build_fts_query("foo bar"), "\"foo\"* AND \"bar\"*");
        // A leading double-quote is escaped by doubling it for FTS5
        assert_eq!(build_fts_query("\"foo"), "\"\"\"foo\"*");
        // An embedded double-quote is also doubled
        assert_eq!(build_fts_query("foo\"bar"), "\"foo\"\"bar\"*");
        assert_eq!(build_fts_query(""), "");
        assert_eq!(build_fts_query("  "), "");
    }

    #[test]
    fn test_name_match_tier() {
        // Exact match
        assert_eq!(name_match_tier("process_data", "process_data"), 3.0);
        assert_eq!(name_match_tier("Process_Data", "process_data"), 3.0);

        // Prefix match
        assert_eq!(name_match_tier("process_data", "process"), 2.0);
        assert_eq!(name_match_tier("Process_Data", "PROCESS"), 2.0);

        // Substring match
        assert_eq!(name_match_tier("my_process_data", "process"), 1.0);

        // Other (matched by FTS5 but not by name)
        assert_eq!(name_match_tier("foo", "bar"), 0.0);
    }
}
