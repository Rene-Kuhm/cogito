//! Decision Log Engine
//!
//! Structured reasoning that survives sessions.

use crate::types::Decision;
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

/// In-memory storage for decisions (TODO: replace with proper storage)
static DECISIONS: once_cell::sync::Lazy<parking_lot::RwLock<HashMap<Uuid, Vec<Decision>>>> =
    once_cell::sync::Lazy::new(|| parking_lot::RwLock::new(HashMap::new()));

/// Add a new decision
pub async fn add(decision: Decision) -> Result<Decision> {
    let mut decisions = DECISIONS.write();
    let project_decisions = decisions.entry(decision.project_id).or_insert_with(Vec::new);
    project_decisions.push(decision.clone());
    Ok(decision)
}

/// Search decisions by query
pub async fn search(project_id: Uuid, query: &str) -> Result<Vec<Decision>> {
    let query_lower = query.to_lowercase();
    let decisions = DECISIONS.read();
    
    let project_decisions = decisions.get(&project_id).cloned().unwrap_or_default();
    
    let results: Vec<Decision> = project_decisions
        .into_iter()
        .filter(|decision| {
            decision.title.to_lowercase().contains(&query_lower)
                || decision.why.to_lowercase().contains(&query_lower)
                || decision.tags.iter().any(|t| t.to_lowercase().contains(&query_lower))
                || decision.paths.iter().any(|p| {
                    p.to_string_lossy()
                        .to_lowercase()
                        .contains(&query_lower)
                })
                || decision.alternatives.iter().any(|a| {
                    a.to_lowercase().contains(&query_lower)
                })
        })
        .collect();

    Ok(results)
}

/// Get all decisions for a project
pub async fn get_all(project_id: Uuid) -> Result<Vec<Decision>> {
    let decisions = DECISIONS.read();
    Ok(decisions.get(&project_id).cloned().unwrap_or_default())
}

/// Get decision by ID
pub async fn get(project_id: Uuid, decision_id: Uuid) -> Result<Option<Decision>> {
    let decisions = DECISIONS.read();
    Ok(decisions
        .get(&project_id)
        .and_then(|d| d.iter().find(|dec| dec.id == decision_id))
        .cloned())
}

/// Delete a decision
pub async fn delete(project_id: Uuid, decision_id: Uuid) -> Result<bool> {
    let mut decisions = DECISIONS.write();
    if let Some(project_decisions) = decisions.get_mut(&project_id) {
        let len_before = project_decisions.len();
        project_decisions.retain(|d| d.id != decision_id);
        return Ok(project_decisions.len() < len_before);
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

    #[tokio::test]
    async fn test_add_and_search() {
        let project_id = Uuid::new_v4();
        
        let decision = Decision {
            id: Uuid::new_v4(),
            project_id,
            title: "JWT with refresh tokens".to_string(),
            why: "stateless, scales better".to_string(),
            alternatives: vec!["Sessions with cookies".to_string()],
            paths: vec![std::path::PathBuf::from("src/auth/")],
            consequences: vec!["Must implement token refresh".to_string()],
            tags: vec!["auth".to_string(), "jwt".to_string()],
            created_at: Utc::now(),
            author: "test".to_string(),
        };

        let added = add(decision.clone()).await.unwrap();
        assert_eq!(added.id, decision.id);

        let results = search(project_id, "JWT").await.unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].title, "JWT with refresh tokens");
    }

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
