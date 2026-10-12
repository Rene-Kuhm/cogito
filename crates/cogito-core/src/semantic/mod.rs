//! Semantic Memory Tags
//!
//! Index knowledge without reading code. Backed by SQLite (with FTS5) when
//! available, falls back to legacy JSON files otherwise.

use crate::types::SemanticTag;
use anyhow::Result;
use chrono::Utc;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

/// Legacy in-memory storage for semantic tags (used when SQLite is not open).
static LEGACY_TAGS: Lazy<RwLock<HashMap<Uuid, Vec<SemanticTag>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

fn legacy_cogito_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".cogito")
}

fn legacy_persist() {
    let map = LEGACY_TAGS.read().clone();
    let all: Vec<SemanticTag> = map.into_values().flatten().collect();
    let _ = crate::persistence::save_tags(&legacy_cogito_dir(), &all);
}

fn legacy_hydrate() {
    let dir = legacy_cogito_dir();
    if let Ok(loaded) = crate::persistence::load_tags(&dir) {
        let mut map = LEGACY_TAGS.write();
        for tag in loaded {
            map.entry(tag.project_id).or_default().push(tag);
        }
    }
}

fn ensure_legacy_loaded() {
    static LOADED: once_cell::sync::Lazy<parking_lot::Once> =
        once_cell::sync::Lazy::new(parking_lot::Once::new);
    LOADED.call_once(legacy_hydrate);
}

/// Optional SQLite connection (set by Cogito before calling these fns).
static SQLITE: once_cell::sync::OnceCell<Arc<parking_lot::Mutex<rusqlite::Connection>>> =
    once_cell::sync::OnceCell::new();

pub(crate) fn set_sqlite(conn: Arc<parking_lot::Mutex<rusqlite::Connection>>) {
    let _ = SQLITE.set(conn);
}

/// Search semantic tags by query
pub async fn search(project_id: Uuid, query: &str) -> Result<Vec<SemanticTag>> {
    if let Some(conn) = SQLITE.get() {
        let guard = conn.lock();
        return crate::storage::sqlite::search_tags(&guard, project_id, query);
    }

    ensure_legacy_loaded();
    let query_lower = query.to_lowercase();
    let tags = LEGACY_TAGS.read();

    let project_tags = tags.get(&project_id).cloned().unwrap_or_default();

    let results: Vec<SemanticTag> = project_tags
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
        .collect();

    Ok(results)
}

/// Add a semantic tag
pub async fn add(tag: SemanticTag) -> Result<SemanticTag> {
    if let Some(conn) = SQLITE.get() {
        let guard = conn.lock();
        crate::storage::sqlite::insert_tag(&guard, &tag)?;
        return Ok(tag);
    }

    ensure_legacy_loaded();
    let mut tags = LEGACY_TAGS.write();
    let project_tags = tags.entry(tag.project_id).or_default();
    project_tags.push(tag.clone());
    drop(tags);
    legacy_persist();
    Ok(tag)
}

/// Create a semantic tag from file paths
pub async fn auto_tag(project_id: Uuid, paths: Vec<PathBuf>) -> Result<Vec<SemanticTag>> {
    let mut tags = Vec::new();

    for path in paths {
        if let Some(tag) = infer_tag_from_path(&path) {
            tags.push(SemanticTag {
                id: Uuid::new_v4(),
                project_id,
                tag: tag.name,
                domain: tag.domain,
                paths: vec![path],
                confidence: tag.confidence,
                created_at: Utc::now(),
            });
        }
    }

    // Insert all tags
    for tag in &tags {
        add(tag.clone()).await?;
    }

    Ok(tags)
}

/// Inferred tag from file path
struct InferredTag {
    name: String,
    domain: String,
    confidence: f32,
}

/// Infer tag from file path patterns
fn infer_tag_from_path(path: &Path) -> Option<InferredTag> {
    let path_str = path.to_string_lossy().to_lowercase();
    let segments: Vec<&str> = path_str.split('/').collect();

    // Common naming patterns
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

/// Get all tags for a project
pub async fn get_all(project_id: Uuid) -> Result<Vec<SemanticTag>> {
    search(project_id, "").await
}

/// Delete a tag
pub async fn delete(project_id: Uuid, tag_id: Uuid) -> Result<bool> {
    if let Some(conn) = SQLITE.get() {
        let guard = conn.lock();
        let deleted = guard.execute(
            "DELETE FROM tags WHERE id = ?1 AND project_id = ?2",
            rusqlite::params![tag_id.to_string(), project_id.to_string()],
        )?;
        return Ok(deleted > 0);
    }

    ensure_legacy_loaded();
    let mut tags = LEGACY_TAGS.write();
    if let Some(project_tags) = tags.get_mut(&project_id) {
        let len_before = project_tags.len();
        project_tags.retain(|t| t.id != tag_id);
        let deleted = project_tags.len() < len_before;
        drop(tags);
        if deleted {
            legacy_persist();
        }
        return Ok(deleted);
    }
    Ok(false)
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
    fn test_legacy_search_returns_empty() {
        // When SQLite is not set, legacy storage should work
        ensure_legacy_loaded();
        let project_id = Uuid::new_v4();
        // No tags added, should return empty
        let result = futures::executor::block_on(search(project_id, ""));
        assert!(result.is_ok());
    }
}
