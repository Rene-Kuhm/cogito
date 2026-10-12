//! Semantic Memory Tags
//!
//! Index knowledge without reading code. Backed by SQLite (with FTS5) when a
//! connection is provided, falls back to legacy JSON files otherwise.
//!
//! ## Why the connection is a parameter
//!
//! Earlier versions kept the SQLite connection in a process-global `OnceCell`.
//! That is wrong when one process serves several projects (for example the
//! MCP server): the first project would win and every later call would read
//! and write the wrong database. The connection and the project's `.cogito`
//! directory are therefore passed explicitly by [`crate::Cogito`].

use crate::types::SemanticTag;
use anyhow::Result;
use chrono::Utc;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Legacy in-memory storage (used only when no SQLite connection is provided,
/// e.g. in unit tests that exercise the pure logic).
static LEGACY_TAGS: Lazy<RwLock<HashMap<Uuid, Vec<SemanticTag>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Read the legacy store.
fn legacy_read() -> HashMap<Uuid, Vec<SemanticTag>> {
    LEGACY_TAGS.read().clone()
}

/// Write the legacy store to `cogito_dir/tags.json`.
fn legacy_persist(cogito_dir: &Path) {
    let all: Vec<SemanticTag> = legacy_read().into_values().flatten().collect();
    let _ = crate::persistence::save_tags(cogito_dir, &all);
}

/// Search semantic tags by query.
pub fn search(
    conn: Option<&Connection>,
    project_id: Uuid,
    query: &str,
) -> Result<Vec<SemanticTag>> {
    if let Some(conn) = conn {
        return crate::storage::sqlite::search_tags(conn, project_id, query);
    }

    let query_lower = query.to_lowercase();
    let project_tags = legacy_read().get(&project_id).cloned().unwrap_or_default();

    Ok(project_tags
        .into_iter()
        .filter(|tag| {
            if query.is_empty() {
                return true;
            }
            tag.tag.to_lowercase().contains(&query_lower)
                || tag.domain.to_lowercase().contains(&query_lower)
                || tag
                    .paths
                    .iter()
                    .any(|p| p.to_string_lossy().to_lowercase().contains(&query_lower))
        })
        .collect())
}

/// Add a semantic tag.
pub fn add(conn: Option<&Connection>, cogito_dir: &Path, tag: SemanticTag) -> Result<SemanticTag> {
    if let Some(conn) = conn {
        crate::storage::sqlite::insert_tag(conn, &tag)?;
        return Ok(tag);
    }

    {
        let mut tags = LEGACY_TAGS.write();
        tags.entry(tag.project_id).or_default().push(tag.clone());
    }
    legacy_persist(cogito_dir);
    Ok(tag)
}

/// Create semantic tags from file paths, inferring the tag name from the path.
pub fn auto_tag(
    conn: Option<&Connection>,
    cogito_dir: &Path,
    project_id: Uuid,
    paths: Vec<PathBuf>,
) -> Result<Vec<SemanticTag>> {
    let mut tags = Vec::new();

    for path in paths {
        if let Some(inferred) = infer_tag_from_path(&path) {
            tags.push(SemanticTag {
                id: Uuid::new_v4(),
                project_id,
                tag: inferred.name,
                domain: inferred.domain,
                paths: vec![path],
                confidence: inferred.confidence,
                created_at: Utc::now(),
            });
        }
    }

    for tag in &tags {
        add(conn, cogito_dir, tag.clone())?;
    }

    Ok(tags)
}

/// Delete a tag by id.
pub fn delete(
    conn: Option<&Connection>,
    cogito_dir: &Path,
    project_id: Uuid,
    tag_id: Uuid,
) -> Result<bool> {
    if let Some(conn) = conn {
        let deleted = conn.execute(
            "DELETE FROM tags WHERE id = ?1 AND project_id = ?2",
            rusqlite::params![tag_id.to_string(), project_id.to_string()],
        )?;
        return Ok(deleted > 0);
    }

    let mut tags = LEGACY_TAGS.write();
    if let Some(project_tags) = tags.get_mut(&project_id) {
        let before = project_tags.len();
        project_tags.retain(|t| t.id != tag_id);
        let deleted = project_tags.len() < before;
        drop(tags);
        if deleted {
            legacy_persist(cogito_dir);
        }
        return Ok(deleted);
    }
    Ok(false)
}

/// Get all tags for a project.
pub fn get_all(conn: Option<&Connection>, project_id: Uuid) -> Result<Vec<SemanticTag>> {
    search(conn, project_id, "")
}

/// Inferred tag from a file path.
struct InferredTag {
    name: String,
    domain: String,
    confidence: f32,
}

/// Infer a tag from file path patterns.
fn infer_tag_from_path(path: &Path) -> Option<InferredTag> {
    let path_str = path.to_string_lossy().to_lowercase();
    let segments: Vec<&str> = path_str.split('/').collect();

    let tag_patterns: HashMap<&str, (&str, &str, f32)> = HashMap::from([
        ("auth", ("auth", "security", 0.9)),
        ("login", ("auth", "security", 0.8)),
        ("signup", ("auth", "security", 0.8)),
        ("password", ("auth", "security", 0.85)),
        ("session", ("auth", "security", 0.8)),
        ("jwt", ("auth", "security", 0.95)),
        ("oauth", ("auth", "security", 0.95)),
        ("billing", ("billing", "commerce", 0.9)),
        ("payment", ("payment", "commerce", 0.9)),
        ("invoice", ("billing", "commerce", 0.9)),
        ("subscription", ("billing", "commerce", 0.9)),
        ("price", ("pricing", "commerce", 0.8)),
        ("stripe", ("payment", "commerce", 0.95)),
        ("api", ("api", "network", 0.85)),
        ("route", ("api", "network", 0.8)),
        ("endpoint", ("api", "network", 0.85)),
        ("controller", ("api", "network", 0.8)),
        ("handler", ("api", "network", 0.75)),
        ("db", ("database", "storage", 0.85)),
        ("repo", ("repository", "storage", 0.85)),
        ("model", ("model", "data", 0.7)),
        ("schema", ("schema", "data", 0.85)),
        ("migration", ("migration", "data", 0.9)),
        ("component", ("ui", "presentation", 0.8)),
        ("page", ("page", "presentation", 0.75)),
        ("layout", ("layout", "presentation", 0.8)),
        ("modal", ("ui", "presentation", 0.85)),
        ("button", ("ui", "presentation", 0.7)),
        ("config", ("config", "infrastructure", 0.9)),
        ("env", ("config", "infrastructure", 0.85)),
        ("constant", ("config", "infrastructure", 0.75)),
    ]);

    for segment in &segments {
        let segment_str: &str = segment;
        if let Some((name, domain, confidence)) = tag_patterns.get(segment_str) {
            return Some(InferredTag {
                name: name.to_string(),
                domain: domain.to_string(),
                confidence: *confidence,
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_tag_from_path() {
        let auth_path = PathBuf::from("/src/auth/jwt.rs");
        let tag = infer_tag_from_path(&auth_path);
        assert!(tag.is_some());
        assert_eq!(tag.unwrap().name, "auth");
    }

    #[test]
    fn test_search_without_connection_uses_legacy_store() {
        let project_id = Uuid::new_v4();
        let result = search(None, project_id, "");
        assert!(result.is_ok());
    }
}
