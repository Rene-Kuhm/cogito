//! Decision Log Engine
//!
//! Structured reasoning that survives sessions. Backed by SQLite (with FTS5 +
//! porter stemming) when a connection is provided, falls back to legacy JSON
//! files otherwise.
//!
//! See [`crate::semantic`] for why the connection is an explicit parameter
//! instead of a process-global.

use crate::types::Decision;
use anyhow::Result;
use chrono::Utc;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Legacy in-memory storage (used only when no SQLite connection is provided).
static LEGACY_DECISIONS: Lazy<RwLock<HashMap<Uuid, Vec<Decision>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

fn legacy_read() -> HashMap<Uuid, Vec<Decision>> {
    LEGACY_DECISIONS.read().clone()
}

fn legacy_persist(cogito_dir: &Path) {
    let all: Vec<Decision> = legacy_read().into_values().flatten().collect();
    let _ = crate::persistence::save_decisions(cogito_dir, &all);
}

/// Add a new decision.
pub fn add(conn: Option<&Connection>, cogito_dir: &Path, decision: Decision) -> Result<Decision> {
    if let Some(conn) = conn {
        crate::storage::sqlite::insert_decision(conn, &decision)?;
        return Ok(decision);
    }

    {
        let mut decisions = LEGACY_DECISIONS.write();
        decisions
            .entry(decision.project_id)
            .or_default()
            .push(decision.clone());
    }
    legacy_persist(cogito_dir);
    Ok(decision)
}

/// Search decisions by query.
pub fn search(conn: Option<&Connection>, project_id: Uuid, query: &str) -> Result<Vec<Decision>> {
    if let Some(conn) = conn {
        return crate::storage::sqlite::search_decisions(conn, project_id, query);
    }

    let query_lower = query.to_lowercase();
    let project_decisions = legacy_read().get(&project_id).cloned().unwrap_or_default();

    Ok(project_decisions
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
        .collect())
}

/// Get all decisions for a project.
pub fn get_all(conn: Option<&Connection>, project_id: Uuid) -> Result<Vec<Decision>> {
    search(conn, project_id, "")
}

/// Get a single decision by id.
pub fn get(
    conn: Option<&Connection>,
    project_id: Uuid,
    decision_id: Uuid,
) -> Result<Option<Decision>> {
    Ok(get_all(conn, project_id)?
        .into_iter()
        .find(|d| d.id == decision_id))
}

/// Delete a decision by id.
pub fn delete(
    conn: Option<&Connection>,
    cogito_dir: &Path,
    project_id: Uuid,
    decision_id: Uuid,
) -> Result<bool> {
    if let Some(conn) = conn {
        let deleted = conn.execute(
            "DELETE FROM decisions WHERE id = ?1 AND project_id = ?2",
            rusqlite::params![decision_id.to_string(), project_id.to_string()],
        )?;
        return Ok(deleted > 0);
    }

    let mut decisions = LEGACY_DECISIONS.write();
    if let Some(project_decisions) = decisions.get_mut(&project_id) {
        let before = project_decisions.len();
        project_decisions.retain(|d| d.id != decision_id);
        let deleted = project_decisions.len() < before;
        drop(decisions);
        if deleted {
            legacy_persist(cogito_dir);
        }
        return Ok(deleted);
    }
    Ok(false)
}

/// Create a builder for new decisions.
pub fn builder(project_id: Uuid) -> DecisionBuilder {
    DecisionBuilder::new(project_id)
}

/// Builder for decisions.
#[derive(Debug)]
pub struct DecisionBuilder {
    project_id: Uuid,
    title: String,
    why: String,
    alternatives: Vec<String>,
    paths: Vec<PathBuf>,
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

    pub fn path(mut self, path: PathBuf) -> Self {
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
