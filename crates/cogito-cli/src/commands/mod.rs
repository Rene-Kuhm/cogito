//! CLI Commands
//!
//! Connects to the cogito-core engine and provides user-facing operations.

use anyhow::{Context, Result};
use cogito_core::{Cogito, CompressionOptions, Project};
use std::path::PathBuf;
use uuid::Uuid;

use crate::DecisionCommands;
use crate::ProjectCommands;
use crate::TagCommands;

// ============================================================
// Project Commands
// ============================================================

/// Initialize Cogito in the current directory
pub async fn init() -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;

    println!("Initializing Cogito in: {}", cwd.display());

    let cogito = Cogito::new();
    let project = cogito
        .init_project(cwd.clone())
        .context("Failed to initialize project")?;

    println!("✓ Project created");
    println!("  ID:      {}", project.id);
    println!("  Name:    {}", project.name);
    println!("  Language: {}", project.language);
    println!();
    println!("Next steps:");
    println!("  cogito scan        # Generate fingerprint");
    println!("  cogito fingerprint # View current fingerprint");

    Ok(())
}

/// Scan project and generate fingerprint
pub async fn scan(_watch: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let project = load_or_create_project(&cwd)?;

    println!("Scanning: {} ({})", project.name, project.language);
    println!();

    let cogito = Cogito::new();
    let fingerprint = cogito
        .generate_fingerprint(&project)
        .await
        .context("Failed to generate fingerprint")?;

    println!("📍 Entry Points ({}):", fingerprint.entry_points.len());
    for ep in fingerprint.entry_points.iter().take(10) {
        let display = ep
            .strip_prefix(&project.path)
            .unwrap_or(ep)
            .display()
            .to_string();
        println!("   • {}", display);
    }
    if fingerprint.entry_points.len() > 10 {
        println!("   ... and {} more", fingerprint.entry_points.len() - 10);
    }
    println!();

    println!("🔀 Data Flows ({}):", fingerprint.data_flows.len());
    let unique_targets: std::collections::HashSet<_> = fingerprint
        .data_flows
        .iter()
        .map(|f| f.to.clone())
        .collect();
    for target in unique_targets.iter().take(10) {
        println!("   → {}", target.display());
    }
    println!();

    println!("🔥 Hot Zones ({}):", fingerprint.hot_zones.len());
    for zone in fingerprint.hot_zones.iter().take(5) {
        let display = zone
            .path
            .strip_prefix(&project.path)
            .unwrap_or(&zone.path)
            .display()
            .to_string();
        println!("   • {} ({} changes)", display, zone.change_count);
    }
    println!();

    println!("❄️  Cold Zones ({}):", fingerprint.cold_zones.len());
    for zone in fingerprint.cold_zones.iter().take(5) {
        let display = zone
            .path
            .strip_prefix(&project.path)
            .unwrap_or(&zone.path)
            .display()
            .to_string();
        println!("   • {} ({} changes)", display, zone.change_count);
    }
    println!();

    println!("🧊 Frozen Zones ({}):", fingerprint.frozen_zones.len());
    for zone in fingerprint.frozen_zones.iter().take(5) {
        let display = zone
            .path
            .strip_prefix(&project.path)
            .unwrap_or(&zone.path)
            .display()
            .to_string();
        println!("   • {}", display);
    }
    println!();

    println!("════════════════════════════════════════");
    println!("Generated: {}", fingerprint.generated_at);

    Ok(())
}

/// Show current fingerprint (stub - reuses scan for now)
pub async fn fingerprint(verbose: bool) -> Result<()> {
    if verbose {
        scan(false).await
    } else {
        println!("Use 'cogito scan' to regenerate the fingerprint.");
        Ok(())
    }
}

// ============================================================
// Context Commands
// ============================================================

/// Compress context
pub async fn compress(
    file: &str,
    max_tokens: usize,
    preserve_imports: bool,
    preserve_types: bool,
    json: bool,
) -> Result<()> {
    let content = if file == "-" {
        let mut buffer = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer)
            .context("Failed to read from stdin")?;
        buffer
    } else {
        std::fs::read_to_string(file).with_context(|| format!("Failed to read file: {file}"))?
    };

    let options = CompressionOptions {
        max_tokens,
        preserve_imports,
        preserve_types,
        hot_zones_priority: true,
    };

    let cogito = Cogito::new();
    let result = cogito
        .compress_context(&content, options)
        .await
        .context("Failed to compress context")?;

    if json {
        let json_output = serde_json::json!({
            "original_tokens": result.original_tokens,
            "compressed_tokens": result.compressed_tokens,
            "savings_percent": result.savings_percent,
            "content": result.content,
            "sources": result.sources,
        });
        println!("{}", serde_json::to_string_pretty(&json_output)?);
    } else {
        println!("Original tokens:  {}", result.original_tokens);
        println!("Compressed tokens: {}", result.compressed_tokens);
        println!("Savings:           {:.1}%", result.savings_percent);
        println!();
        println!("--- Compressed Content ---");
        println!("{}", result.content);
    }

    Ok(())
}

/// Query knowledge base
pub async fn query(query_str: &str, json: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let project = load_or_create_project(&cwd)?;

    let cogito = Cogito::new();
    let tags = cogito
        .search_tags(project.id, query_str)
        .await
        .context("Failed to search tags")?;
    let decisions = cogito
        .search_decisions(project.id, query_str)
        .await
        .context("Failed to search decisions")?;

    if json {
        let output = serde_json::json!({
            "query": query_str,
            "tags": tags,
            "decisions": decisions,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Search: {}", query_str);
        println!();
        println!("📌 Tags ({}):", tags.len());
        for tag in &tags {
            println!("   • {} ({})", tag.tag, tag.domain);
        }
        println!();
        println!("📋 Decisions ({}):", decisions.len());
        for decision in &decisions {
            println!("   • {} - {}", decision.title, decision.why);
        }
    }

    Ok(())
}

// ============================================================
// Decision Commands
// ============================================================

pub async fn decision(subcommand: Option<DecisionCommands>) -> Result<()> {
    match subcommand {
        Some(DecisionCommands::Add {
            title,
            why,
            alternative,
            paths,
            tags,
        }) => add_decision(title, why, alternative, paths, tags).await,
        Some(DecisionCommands::List) => list_decisions().await,
        Some(DecisionCommands::Search { query }) => search_decisions(&query).await,
        Some(DecisionCommands::Show { id }) => show_decision(&id).await,
        None => {
            println!("Usage: cogito decision <add|list|search|show>");
            Ok(())
        }
    }
}

async fn add_decision(
    title: Option<String>,
    why: Option<String>,
    alternatives: Vec<String>,
    paths: Vec<String>,
    tags: Vec<String>,
) -> Result<()> {
    let (title_str, why_str) = match (title, why) {
        (Some(t), Some(w)) => (t, w),
        _ => {
            println!("Error: --title and --why are required");
            println!("Usage: cogito decision add --title '...' --why '...'");
            return Ok(());
        }
    };

    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let project = load_or_create_project(&cwd)?;

    let cogito = Cogito::new();

    // Build the decision with all options
    let mut builder = cogito_core::decision::builder(project.id)
        .title(&title_str)
        .why(&why_str)
        .author(&whoami::username());
    for alt in &alternatives {
        builder = builder.alternative(alt);
    }
    for tag in &tags {
        builder = builder.tag(tag);
    }
    for path_str in &paths {
        builder = builder.path(PathBuf::from(path_str));
    }
    let decision = builder.build();

    let added = cogito
        .add_decision(decision)
        .await
        .context("Failed to add decision")?;

    println!("✓ Decision added");
    println!("  ID:    {}", added.id);
    println!("  Title: {}", added.title);
    println!("  Why:   {}", added.why);
    if !added.alternatives.is_empty() {
        println!("  Alternatives:");
        for alt in &added.alternatives {
            println!("    - {}", alt);
        }
    }
    if !added.tags.is_empty() {
        println!("  Tags:  {}", added.tags.join(", "));
    }

    Ok(())
}

async fn list_decisions() -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let project = load_or_create_project(&cwd)?;

    let cogito = Cogito::new();
    // For now, return all by searching empty
    let decisions = cogito
        .search_decisions(project.id, "")
        .await
        .context("Failed to list decisions")?;

    if decisions.is_empty() {
        println!("No decisions found.");
        println!("Add one with: cogito decision add --title '...' --why '...'");
        return Ok(());
    }

    println!("Decisions ({}):", decisions.len());
    for d in &decisions {
        println!("  • {} - {}", d.title, d.why);
    }
    Ok(())
}

async fn search_decisions(query_str: &str) -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let project = load_or_create_project(&cwd)?;

    let cogito = Cogito::new();
    let decisions = cogito
        .search_decisions(project.id, query_str)
        .await
        .context("Failed to search decisions")?;

    if decisions.is_empty() {
        println!("No decisions found matching: {}", query_str);
        return Ok(());
    }

    println!("Decisions matching '{}' ({}):", query_str, decisions.len());
    for d in &decisions {
        println!("  • {} - {}", d.title, d.why);
    }
    Ok(())
}

async fn show_decision(id: &str) -> Result<()> {
    println!("Showing decision: {}", id);
    println!("(not yet implemented - coming soon)");
    Ok(())
}

// ============================================================
// Tag Commands
// ============================================================

pub async fn tag(subcommand: Option<TagCommands>) -> Result<()> {
    match subcommand {
        Some(TagCommands::Add { tag, paths }) => add_tag(&tag, paths).await,
        Some(TagCommands::List) => list_tags().await,
        Some(TagCommands::Search { query }) => search_tags(&query).await,
        None => {
            println!("Usage: cogito tag <add|list|search>");
            Ok(())
        }
    }
}

async fn add_tag(tag: &str, paths: Vec<String>) -> Result<()> {
    if paths.is_empty() {
        println!("Error: at least one path is required");
        println!("Usage: cogito tag add <tag> <paths...>");
        return Ok(());
    }

    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let project = load_or_create_project(&cwd)?;

    let cogito = Cogito::new();
    let semantic_tag = cogito_core::types::SemanticTag {
        id: Uuid::new_v4(),
        project_id: project.id,
        tag: tag.to_string(),
        domain: "manual".to_string(),
        paths: paths.iter().map(PathBuf::from).collect(),
        confidence: 1.0,
        created_at: chrono::Utc::now(),
    };

    let added = cogito
        .add_tag(semantic_tag)
        .await
        .context("Failed to add tag")?;

    println!("✓ Tag added");
    println!("  Tag:  {}", added.tag);
    println!("  Paths: {}", added.paths.len());

    Ok(())
}

async fn list_tags() -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let project = load_or_create_project(&cwd)?;

    let cogito = Cogito::new();
    let tags = cogito
        .search_tags(project.id, "")
        .await
        .context("Failed to list tags")?;

    if tags.is_empty() {
        println!("No tags found.");
        println!("Add one with: cogito tag add <tag> <paths...>");
        return Ok(());
    }

    println!("Tags ({}):", tags.len());
    for tag in &tags {
        println!(
            "  • {} ({}) - {} paths",
            tag.tag,
            tag.domain,
            tag.paths.len()
        );
    }
    Ok(())
}

async fn search_tags(query_str: &str) -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let project = load_or_create_project(&cwd)?;

    let cogito = Cogito::new();
    let tags = cogito
        .search_tags(project.id, query_str)
        .await
        .context("Failed to search tags")?;

    if tags.is_empty() {
        println!("No tags found matching: {}", query_str);
        return Ok(());
    }

    println!("Tags matching '{}' ({}):", query_str, tags.len());
    for tag in &tags {
        println!(
            "  • {} ({}) - {} paths",
            tag.tag,
            tag.domain,
            tag.paths.len()
        );
    }
    Ok(())
}

// ============================================================
// Project Subcommands
// ============================================================

pub async fn project(subcommand: Option<ProjectCommands>) -> Result<()> {
    match subcommand {
        Some(ProjectCommands::List) => list_projects().await,
        Some(ProjectCommands::Delete { id }) => delete_project(&id).await,
        None => {
            println!("Usage: cogito project <list|delete>");
            Ok(())
        }
    }
}

async fn list_projects() -> Result<()> {
    println!("Projects (in-memory only - persistence coming soon):");
    println!("  (none - run 'cogito init' in a project directory)");
    Ok(())
}

async fn delete_project(_id: &str) -> Result<()> {
    println!("Delete project: not yet implemented");
    Ok(())
}

// ============================================================
// Server Commands
// ============================================================

/// Start server mode
pub async fn serve(port: u16, mcp: bool) -> Result<()> {
    if mcp {
        println!("Starting MCP server on port {}...", port);
        // TODO: Start MCP server
    } else {
        println!("Starting REST API server on port {}...", port);
        println!("Run 'cogito-api' to start the API server");
    }
    println!();
    println!("Server started. Press Ctrl+C to stop.");

    // Keep running
    tokio::signal::ctrl_c()
        .await
        .context("Failed to wait for Ctrl+C")?;
    Ok(())
}

// ============================================================
// Stats / Doctor / Export
// ============================================================

/// Show statistics
pub async fn stats(json: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let project = load_or_create_project(&cwd)?;

    let cogito = Cogito::new();
    let tags = cogito.search_tags(project.id, "").await.unwrap_or_default();
    let decisions = cogito
        .search_decisions(project.id, "")
        .await
        .unwrap_or_default();

    if json {
        let stats = serde_json::json!({
            "project": project.name,
            "tags": tags.len(),
            "decisions": decisions.len(),
        });
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("=== Cogito Statistics ===");
        println!("Project:   {}", project.name);
        println!("Tags:      {}", tags.len());
        println!("Decisions: {}", decisions.len());
    }
    Ok(())
}

/// Diagnose configuration
pub async fn doctor() -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;

    println!("Running diagnostics...");
    println!();

    // Check project directory
    println!("  Project: {}", cwd.display());

    // Check git
    let has_git = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("--git-dir")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    println!(
        "  Git:     {}",
        if has_git {
            "✓ found"
        } else {
            "○ not a git repo (zone detection will be limited)"
        }
    );

    // Check for common project files
    let project_files = [
        ("Cargo.toml", "Rust project"),
        ("package.json", "Node.js project"),
        ("go.mod", "Go project"),
        ("pyproject.toml", "Python project"),
        ("Gemfile", "Ruby project"),
        ("composer.json", "PHP project"),
    ];
    for (file, desc) in project_files {
        if cwd.join(file).exists() {
            println!("  Project file: ✓ {} ({})", file, desc);
        }
    }

    println!();
    println!("✓ Diagnostics complete");
    Ok(())
}

/// Export project data
pub async fn export(output: Option<&str>, format: &str) -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let project = load_or_create_project(&cwd)?;

    let cogito = Cogito::new();
    let fingerprint = cogito.generate_fingerprint(&project).await?;
    let tags = cogito.search_tags(project.id, "").await?;
    let decisions = cogito.search_decisions(project.id, "").await?;

    let export_data = serde_json::json!({
        "version": "0.1.0",
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "project": {
            "id": project.id,
            "name": project.name,
            "path": project.path,
            "language": project.language,
        },
        "fingerprint": {
            "entry_points": fingerprint.entry_points.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "data_flows_count": fingerprint.data_flows.len(),
            "hot_zones_count": fingerprint.hot_zones.len(),
            "cold_zones_count": fingerprint.cold_zones.len(),
            "frozen_zones_count": fingerprint.frozen_zones.len(),
        },
        "tags": tags,
        "decisions": decisions,
    });

    let json_output = if format == "yaml" {
        // Simple JSON-to-YAML-ish output (not real YAML, but readable)
        format!(
            "# Cogito Export\n# Format: {}\n{}",
            format,
            serde_json::to_string_pretty(&export_data)?
        )
    } else {
        serde_json::to_string_pretty(&export_data)?
    };

    match output {
        Some(path) => {
            std::fs::write(path, &json_output)
                .with_context(|| format!("Failed to write to {path}"))?;
            println!("✓ Exported to {}", path);
        }
        None => println!("{}", json_output),
    }
    Ok(())
}

// ============================================================
// Helpers
// ============================================================

/// Load or create a project for the current directory
fn load_or_create_project(cwd: &std::path::Path) -> Result<Project> {
    let cogito = Cogito::new();
    cogito
        .init_project(cwd.to_path_buf())
        .context("Failed to load/create project")
}

// ProjectId is just a type alias for Uuid
type _ProjectId = Uuid;
