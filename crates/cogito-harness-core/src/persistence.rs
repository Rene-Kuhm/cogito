//! Persistence layer
//!
//! File-based JSON storage for semantic tags and decisions.

use crate::types::{Decision, SemanticTag};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

const TAGS_FILE: &str = "tags.json";
const DECISIONS_FILE: &str = "decisions.json";
const FINGERPRINT_FILE: &str = "fingerprint.json";

/// Load semantic tags from .cogito directory
pub fn load_tags(cogito_dir: &Path) -> Result<Vec<SemanticTag>> {
    let path = cogito_dir.join(TAGS_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let tags: Vec<SemanticTag> = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(tags)
}

/// Save semantic tags to .cogito directory
pub fn save_tags(cogito_dir: &Path, tags: &[SemanticTag]) -> Result<()> {
    fs::create_dir_all(cogito_dir)
        .with_context(|| format!("Failed to create {}", cogito_dir.display()))?;
    let path = cogito_dir.join(TAGS_FILE);
    let content = serde_json::to_string_pretty(tags)?;
    fs::write(&path, content).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// Load decisions from .cogito directory
pub fn load_decisions(cogito_dir: &Path) -> Result<Vec<Decision>> {
    let path = cogito_dir.join(DECISIONS_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let decisions: Vec<Decision> = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(decisions)
}

/// Save decisions to .cogito directory
pub fn save_decisions(cogito_dir: &Path, decisions: &[Decision]) -> Result<()> {
    fs::create_dir_all(cogito_dir)
        .with_context(|| format!("Failed to create {}", cogito_dir.display()))?;
    let path = cogito_dir.join(DECISIONS_FILE);
    let content = serde_json::to_string_pretty(decisions)?;
    fs::write(&path, content).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// Save fingerprint to .cogito directory
pub fn save_fingerprint(cogito_dir: &Path, fingerprint: &crate::types::Fingerprint) -> Result<()> {
    fs::create_dir_all(cogito_dir)?;
    let path = cogito_dir.join(FINGERPRINT_FILE);
    let content = serde_json::to_string_pretty(fingerprint)?;
    fs::write(&path, content)?;
    Ok(())
}

/// Load fingerprint from .cogito directory
pub fn load_fingerprint(cogito_dir: &Path) -> Result<Option<crate::types::Fingerprint>> {
    let path = cogito_dir.join(FINGERPRINT_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)?;
    let fp: crate::types::Fingerprint = serde_json::from_str(&content)?;
    Ok(Some(fp))
}
