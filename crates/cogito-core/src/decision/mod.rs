//! Decision Log Engine
//!
//! Structured reasoning that survives sessions. Backed by SQLite (with FTS5 +
//! porter stemming) when available, falls back to legacy JSON files otherwise.

use crate::types::Decision;
use anyhow::Result;
use chrono::Utc;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

/// Legacy in-memory storage for decisions (used when SQLite is not open).
static LEGACY_DECISIONS: Lazy<RwLock<HashMap<Uuid, Vec<Decision>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

fn legacy_cogito_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".cogito")
}

fn legacy_persist() {
    let map = LEGACY_DECISIONS.read().clone();
    let all: Vec<Decision> = map.into_values().flatten().collect();
    let _ = crate::persistence::save_decisions(&legacy_cogito_dir(), &all);
}

fn legacy_hydrate() {
    let dir = legacy_cogito_dir();
    if let Ok(loaded) = crate::persistence::load_decisions(&dir) {
        let mut map = LEGACY_DECISIONS.write();
        for d in loaded {
            map.entry(d.project_id).or_default().push(d);
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

/// Add a new decision
pub async fn add(decision: Decision) -> Result<Decision> {
    if let Some(conn) = SQLITE.get() {
        let guard = conn.lock();
        crate::storage::sqlite::insert_decision(&guard, &decision)?;
        return Ok(decision);
    }

    ensure_legacy_loaded();
    let mut decisions = LEGACY_DECISIONS.write();
    let project_decisions = decisions.entry(decision.project_id).or_default();
    project_decisions.push(decision.clone());
    drop(decisions);
    legacy_persist();
    Ok(decision)
}

/// Search decisions by query
pub async fn search(project_id: Uuid, query: &str) -> Result<Vec<Decision>> {
    if let Some(conn) = SQLITE.get() {
        let guard = conn.lock();
        return crate::storage::sqlite::search_decisions(&guard, project_id, query);
    }

    ensure_legacy_loaded();
    let query_lower = query.to_lowercase();
    let decisions = LEGACY_DECISIONS.read();

    let project_decisions = decisions.get(&project_id).cloned().unwrap_or_default();

    let results: Vec<Decision> = project_decisions
        .into_iter()
        .filter(|decision| {
            if query.is_empty() {
                return true;
            }
            decision.title.to_lowercase().contains(&query_lower)
                || decision.why.to_lowercase().contains(&query_lower)
                || decision
                    .tags
                    .iter()
                    .any(|t| t.to_lowercase().contains(&query_lower))
                || decision
                    .paths
                    .iter()
                    .any(|p| p.to_string_lossy().to_lowercase().contains(&query_lower))
                || decision
                    .alternatives
                    .iter()
                    .any(|a| a.to_lowercase().contains(&query_lower))
        })
        .collect();

    Ok(results)
}

/// Get all decisions for a project
pub async fn get_all(project_id: Uuid) -> Result<Vec<Decision>> {
    search(project_id, "").await
}

/// Get decision by ID
pub async fn get(project_id: Uuid, decision_id: Uuid) -> Result<Option<Decision>> {
    let results = search(project_id, "").await?;
    Ok(results.into_iter().find(|d| d.id == decision_id))
}

/// Delete a decision
pub async fn delete(project_id: Uuid, decision_id: Uuid) -> Result<bool> {
    if let Some(conn) = SQLITE.get() {
        let guard = conn.lock();
        let deleted = guard.execute(
            "DELETE FROM decisions WHERE id = ?1 AND project_id = ?2",
            rusqlite::params![decision_id.to_string(), project_id.to_string()],
        )?;
        return Ok(deleted > 0);
    }

    ensure_legacy_loaded();
    let mut decisions = LEGACY_DECISIONS.write();
    if let Some(project_decisions) = decisions.get_mut(&project_id) {
        let len_before = project_decisions.len();
        project_decisions.retain(|d| d.id != decision_id);
        let deleted = project_decisions.len() < len_before;
        drop(decisions);
        if deleted {
            legacy_persist();
        }
        return Ok(deleted);
    }
    Ok(false)
}

/// Create a builder for new decisions
pub fn builder(project_id: Uuid) -> DecisionBuilder {
    DecisionBuilder::new(project_id)
}

/// Builder for decisions
#[derive(Debug)]
pub struct DecisionBuilder {
    project_id: Uuid,
    title: String,
    why: String,
    alternatives: Vec<String>,
    paths: Vec<std::path::PathBuf>,
    consequences: Vec<String>,
    tags: Vec<String>,
    author: String,
}

impl DecisionBuilder {
    fn new(project_id: Uuid) -> Self {
        Self {
            project_id,
            title: String::new(),
            why: String::new(),
            alternatives: Vec::new(),
            paths: Vec::new(),
            consequences: Vec::new(),
            tags: Vec::new(),
            author: whoami::username(),
        }
    }

    pub fn title(mut self, title: &str) -> Self {
        self.title = title.to_string();
        self
    }

    pub fn why(mut self, why: &str) -> Self {
        self.why = why.to_string();
        self
    }

    pub fn alternative(mut self, alternative: &str) -> Self {
        self.alternatives.push(alternative.to_string());
        self
    }

    pub fn path(mut self, path: std::path::PathBuf) -> Self {
        self.paths.push(path);
        self
    }

    pub fn consequence(mut self, consequence: &str) -> Self {
        self.consequences.push(consequence.to_string());
        self
    }

    pub fn tag(mut self, tag: &str) -> Self {
        self.tags.push(tag.to_string());
        self
    }

    pub fn author(mut self, author: &str) -> Self {
        self.author = author.to_string();
        self
    }

    pub fn build(self) -> Decision {
        Decision {
            id: Uuid::new_v4(),
            project_id: self.project_id,
            title: self.title,
            why: self.why,
            alternatives: self.alternatives,
            paths: self.paths,
            consequences: self.consequences,
            tags: self.tags,
            created_at: Utc::now(),
            author: self.author,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder() {
        let project_id = Uuid::new_v4();
        let decision = builder(project_id)
            .title("Use PostgreSQL")
            .why("ACID compliance required")
            .alternative("MongoDB")
            .tag("database")
            .build();

        assert_eq!(decision.title, "Use PostgreSQL");
        assert_eq!(decision.why, "ACID compliance required");
        assert!(decision.alternatives.contains(&"MongoDB".to_string()));
        assert!(decision.tags.contains(&"database".to_string()));
    }
}
