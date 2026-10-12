//! Tests for the gentle-shell skill
//!
//! Validates that the skill file is well-formed and follows gentle-shell conventions.

use std::fs;
use std::path::PathBuf;

fn skill_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("SKILL.md")
}

#[test]
fn test_skill_file_exists() {
    assert!(
        skill_path().exists(),
        "Skill file must exist at {}",
        skill_path().display()
    );
}

#[test]
fn test_skill_has_frontmatter() {
    let content = fs::read_to_string(skill_path()).expect("Failed to read SKILL.md");
    assert!(
        content.starts_with("---\n"),
        "SKILL.md must start with YAML frontmatter (---)"
    );
    assert!(
        content.contains("\n---\n"),
        "SKILL.md must have closing frontmatter delimiter"
    );
}

#[test]
fn test_skill_has_required_fields() {
    let content = fs::read_to_string(skill_path()).expect("Failed to read SKILL.md");
    let frontmatter = content
        .split("---")
        .nth(1)
        .expect("Could not extract frontmatter");

    assert!(
        frontmatter.contains("name:"),
        "Skill must have 'name' field"
    );
    assert!(
        frontmatter.contains("description:"),
        "Skill must have 'description' field"
    );
}

#[test]
fn test_skill_name_is_cogito() {
    let content = fs::read_to_string(skill_path()).expect("Failed to read SKILL.md");
    let frontmatter = content.split("---").nth(1).expect("frontmatter missing");

    assert!(
        frontmatter.lines().any(|l| l.trim() == "name: cogito"),
        "Skill name must be 'cogito'"
    );
}

#[test]
fn test_skill_description_is_meaningful() {
    let content = fs::read_to_string(skill_path()).expect("Failed to read SKILL.md");
    let frontmatter = content.split("---").nth(1).expect("frontmatter missing");

    // Pull the description value
    let desc = frontmatter
        .lines()
        .find(|l| l.trim().starts_with("description:"))
        .expect("description field missing");

    assert!(
        desc.len() > 30,
        "Description must be substantial (got {} chars): {desc}",
        desc.len()
    );
}

#[test]
fn test_skill_has_workflow_section() {
    let content = fs::read_to_string(skill_path()).expect("Failed to read SKILL.md");
    assert!(
        content.contains("## Standard Workflow")
            || content.contains("## Workflow")
            || content.contains("## When to Use"),
        "Skill must describe when/why to use it"
    );
}

#[test]
fn test_skill_mentions_integration() {
    let content = fs::read_to_string(skill_path()).expect("Failed to read SKILL.md");
    assert!(
        content.contains("MCP") || content.contains("mcp"),
        "Skill should reference MCP integration"
    );
}
