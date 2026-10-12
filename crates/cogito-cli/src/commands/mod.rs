//! CLI Commands
//!
//! Connects to the cogito-core engine and provides user-facing operations.

use anyhow::{Context, Result};
use cogito_core::{Cogito, CompressionOptions, Project};
use std::path::{Path, PathBuf};
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
    let cogito_dir = cwd.join(".cogito");

    println!("Initializing Cogito in: {}", cwd.display());

    let cogito = open_cogito(&cwd)?;
    let project = cogito
        .init_project(cwd.clone())
        .context("Failed to initialize project")?;

    // Register project in SQLite
    if let Some(sqlite) = cogito.sqlite() {
        let _ = cogito_core::storage::sqlite::upsert_project(
            &sqlite.lock(),
            project.id,
            &project.path,
            &project.name,
            &project.language.to_string(),
        );
    }

    println!("✓ Project created");
    println!("  ID:       {}", project.id);
    println!("  Name:     {}", project.name);
    println!("  Language: {}", project.language);
    println!(
        "  Storage:  SQLite at {}",
        cogito_dir.join("state.db").display()
    );
    println!();
    println!("Next steps:");
    println!("  cogito scan        # Generate fingerprint");
    println!("  cogito fingerprint # View current fingerprint");

    Ok(())
}

/// Scan project and generate fingerprint
pub async fn scan(_watch: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;

    println!("Scanning: {} ({})", project.name, project.language);
    println!();

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
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;

    let tags = cogito
        .search_tags(project.id, query_str)
        .context("Failed to search tags")?;
    let decisions = cogito
        .search_decisions(project.id, query_str)
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
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;

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
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;

    // For now, return all by searching empty
    let decisions = cogito
        .search_decisions(project.id, "")
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
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;

    let decisions = cogito
        .search_decisions(project.id, query_str)
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
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;

    let semantic_tag = cogito_core::types::SemanticTag {
        id: Uuid::new_v4(),
        project_id: project.id,
        tag: tag.to_string(),
        domain: "manual".to_string(),
        paths: paths.iter().map(PathBuf::from).collect(),
        confidence: 1.0,
        created_at: chrono::Utc::now(),
    };

    let added = cogito.add_tag(semantic_tag).context("Failed to add tag")?;

    println!("✓ Tag added");
    println!("  Tag:  {}", added.tag);
    println!("  Paths: {}", added.paths.len());

    Ok(())
}

async fn list_tags() -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;

    let tags = cogito
        .search_tags(project.id, "")
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
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;

    let tags = cogito
        .search_tags(project.id, query_str)
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
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;

    let tags = cogito.search_tags(project.id, "").unwrap_or_default();
    let decisions = cogito.search_decisions(project.id, "").unwrap_or_default();

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

    println!("Cogito diagnostics");
    println!();
    println!("Binary");
    println!("  cogito    {}", env!("CARGO_PKG_VERSION"));
    println!(
        "  path      {}",
        std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "(unknown)".into())
    );

    println!();
    println!("Project");
    println!("  path      {}", cwd.display());

    let has_git = std::process::Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    println!(
        "  git       {}",
        if has_git {
            "found"
        } else {
            "not a repository (zone detection limited to filesystem)"
        }
    );

    let state_db = cwd.join(".cogito/state.db");
    println!(
        "  storage   {}",
        if state_db.exists() {
            format!("SQLite at {}", state_db.display())
        } else {
            "no .cogito/state.db yet (run `cogito scan` or `cogito init`)".to_string()
        }
    );

    let project_files = [
        ("Cargo.toml", "Rust"),
        ("package.json", "Node.js"),
        ("go.mod", "Go"),
        ("pyproject.toml", "Python"),
        ("Gemfile", "Ruby"),
        ("composer.json", "PHP"),
        ("build.gradle.kts", "Kotlin"),
        ("Package.swift", "Swift"),
    ];
    let detected: Vec<&str> = project_files
        .iter()
        .filter(|(f, _)| cwd.join(f).exists())
        .map(|(_, d)| *d)
        .collect();
    println!(
        "  language  {}",
        if detected.is_empty() {
            "unknown".to_string()
        } else {
            detected.join(", ")
        }
    );

    // -- Agent integrations --
    println!();
    println!("Agent integrations");
    let home = crate::agents::home_dir()?;
    let binary = std::env::current_exe().context("Cannot locate the cogito binary")?;
    let mut outdated = 0usize;
    for agent in crate::agents::ALL {
        let status = crate::agents::status(*agent, &home, &binary)?;
        let mark = match status {
            crate::agents::Status::Configured => "✓",
            crate::agents::Status::Outdated => "!",
            crate::agents::Status::NotConfigured => "○",
            crate::agents::Status::AgentMissing => "·",
        };
        if status == crate::agents::Status::Outdated {
            outdated += 1;
        }
        println!("  {mark} {:<20} {}", agent.display(), status.label());
    }

    println!();
    if outdated > 0 {
        println!(
            "{outdated} agent(s) hold a stale entry (a different binary path, or older fields)."
        );
        println!("Re-run `cogito setup` to bring them up to date.");
    } else {
        println!("✓ Diagnostics complete");
    }
    Ok(())
}

// ============================================================
// Agent setup / uninstall
// ============================================================

/// Resolve the list of agents to act on from CLI arguments.
fn resolve_agents(agents: &[String]) -> Result<Vec<crate::agents::Agent>> {
    let mut selected = Vec::new();
    for name in agents {
        match crate::agents::Agent::parse(name) {
            Some(agent) => {
                if !selected.contains(&agent) {
                    selected.push(agent);
                }
            }
            None => {
                eprintln!("Unknown agent: {name}");
                eprintln!(
                    "Known agents: {}",
                    crate::agents::ALL
                        .iter()
                        .map(|a| a.id())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                anyhow::bail!("unknown agent `{name}`");
            }
        }
    }
    Ok(selected)
}

/// Heuristic: does this binary path look like a real install rather than a
/// build artifact?
///
/// `target/debug/cogito` or `target/release/cogito` will move or vanish, which
/// would leave every agent pointing at a broken command.
fn looks_installed(binary: &Path) -> bool {
    let path = binary.to_string_lossy();
    !path.contains("/target/debug/") && !path.contains("/target/release/")
}

/// Install Cogito into the requested (or all detected) agents.
pub fn setup(agents: Vec<String>, all: bool, dry_run: bool) -> Result<()> {
    let home = crate::agents::home_dir()?;
    let binary = std::env::current_exe().context("Cannot locate the cogito binary")?;

    let mut selected = resolve_agents(&agents)?;
    if selected.is_empty() {
        if all {
            selected = crate::agents::ALL
                .iter()
                .copied()
                .filter(|a| a.agent_present(&home))
                .collect();
        } else {
            selected = pick_interactively(&home, &binary)?;
        }
    }

    if selected.is_empty() {
        println!("Nothing selected. Use `cogito setup <agent>` or `cogito setup --all`.");
        return Ok(());
    }

    let mut configured: Vec<crate::agents::Agent> = Vec::new();
    let mut failed: Vec<crate::agents::Agent> = Vec::new();

    println!("Home:   {}", home.display());
    println!("Binary: {}", binary.display());
    if dry_run {
        println!("Mode:   dry run (nothing will be written)");
    }
    if !looks_installed(&binary) {
        println!();
        println!("Warning: this binary is not in a normal install location.");
        println!("         The agents will be configured with the path above, which");
        println!("         may be a build artifact that later disappears or changes.");
        println!("         Install first (`cargo install --path crates/cogito-cli` or");
        println!("         install.sh) and re-run setup from the installed binary.");
    }
    println!();

    for agent in selected {
        if agent.uses_pi_package() {
            println!("  {}:", agent.display());
            println!(
                "    registers the npm package `{}` in {}",
                crate::agents::PI_PACKAGE,
                agent.config_path(&home).display()
            );
            println!(
                "    note: that package is not published yet — the agent will skip it until it is"
            );
        }

        if dry_run {
            let status = crate::agents::status(agent, &home, &binary)?;
            println!(
                "  {} [{}] -> would {}",
                agent.display(),
                status.label(),
                match status {
                    crate::agents::Status::Configured => "stay as is",
                    crate::agents::Status::Outdated => "update the entry",
                    _ => "write",
                }
            );
            continue;
        }

        match crate::agents::install(agent, &home, &binary) {
            Ok(crate::agents::Change::AlreadyCorrect) => {
                println!("  {} — already up to date", agent.display());
                configured.push(agent);
            }
            Ok(crate::agents::Change::Written { backup }) => {
                println!("  {} — configured", agent.display());
                println!("    file:   {}", agent.config_path(&home).display());
                if let Some(b) = backup {
                    println!("    backup: {}", b.display());
                }
                configured.push(agent);
            }
            Err(e) => {
                failed.push(agent);
                println!("  {} — failed", agent.display());
                println!("    file:   {}", agent.config_path(&home).display());
                // Print the full cause chain: "Is a directory" alone does not
                // say which file or which step went wrong.
                for (i, cause) in e.chain().enumerate() {
                    println!("    {}{cause}", if i == 0 { "error: " } else { "cause: " });
                }
            }
        }
    }

    println!();
    if failed.is_empty() {
        println!("Restart the agent(s) for the MCP server to load.");
    } else if configured.is_empty() {
        println!(
            "Nothing was configured: all {} selected agent(s) failed.",
            failed.len()
        );
    } else {
        println!(
            "Configured: {}. Failed: {} — see the errors above.",
            configured
                .iter()
                .map(|a| a.display())
                .collect::<Vec<_>>()
                .join(", "),
            failed
                .iter()
                .map(|a| a.display())
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!("Restart the configured agent(s) for the MCP server to load.");
    }
    Ok(())
}

/// Remove Cogito from the requested (or every configured) agents.
pub fn uninstall(agents: Vec<String>, yes: bool) -> Result<()> {
    let home = crate::agents::home_dir()?;
    let binary = std::env::current_exe().context("Cannot locate the cogito binary")?;

    let mut selected = resolve_agents(&agents)?;
    if selected.is_empty() {
        selected = crate::agents::ALL
            .iter()
            .copied()
            .filter(|a| {
                // Include outdated entries: they are Cogito's and must go too.
                crate::agents::status(*a, &home, &binary)
                    .map(|s| s.is_present())
                    .unwrap_or(false)
            })
            .collect();
    }

    if selected.is_empty() {
        println!("Cogito is not configured in any agent.");
        return Ok(());
    }

    if !yes {
        println!("Cogito will be removed from:");
        for agent in &selected {
            println!(
                "  • {} ({})",
                agent.display(),
                agent.config_path(&home).display()
            );
        }
        print!("\nContinue? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("Aborted.");
            return Ok(());
        }
    }

    println!();
    for agent in selected {
        match crate::agents::uninstall(agent, &home) {
            Ok(crate::agents::Change::AlreadyCorrect) => {
                println!("  {} — nothing to do", agent.display());
            }
            Ok(crate::agents::Change::Written { backup }) => {
                println!("  {} — removed", agent.display());
                if let Some(b) = backup {
                    println!("    backup: {}", b.display());
                }
            }
            Err(e) => println!("  {} — failed: {e}", agent.display()),
        }
    }
    Ok(())
}

/// Ask which detected agents to configure.
fn pick_interactively(home: &Path, binary: &Path) -> Result<Vec<crate::agents::Agent>> {
    let available: Vec<crate::agents::Agent> = crate::agents::ALL
        .iter()
        .copied()
        .filter(|a| a.agent_present(home))
        .collect();

    if available.is_empty() {
        println!("No supported agents detected in {}.\n", home.display());
        println!(
            "Supported agents: {}",
            crate::agents::ALL
                .iter()
                .map(|a| a.id())
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Ok(Vec::new());
    }

    println!("Detected agents:");
    for (i, agent) in available.iter().enumerate() {
        let status = crate::agents::status(*agent, home, binary)?;
        println!("  {}) {:<20} {}", i + 1, agent.display(), status.label());
    }
    println!("  a) all of them");
    print!("\nSelect (comma-separated numbers, or `a`): ");
    use std::io::Write;
    std::io::stdout().flush()?;

    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    let answer = answer.trim().to_lowercase();

    if answer == "a" || answer == "all" {
        return Ok(available);
    }

    let mut chosen = Vec::new();
    for part in answer.split(',') {
        if let Ok(n) = part.trim().parse::<usize>()
            && n >= 1
            && n <= available.len()
        {
            let agent = available[n - 1];
            if !chosen.contains(&agent) {
                chosen.push(agent);
            }
        }
    }
    Ok(chosen)
}

/// Export project data
pub async fn export(output: Option<&str>, format: &str) -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;

    let fingerprint = cogito.generate_fingerprint(&project).await?;
    let tags = cogito.search_tags(project.id, "")?;
    let decisions = cogito.search_decisions(project.id, "")?;

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

/// Open a Cogito instance backed by SQLite at `<cwd>/.cogito/state.db`.
/// Auto-migrates legacy JSON files on first run.
fn open_cogito(cwd: &Path) -> Result<Cogito> {
    let cogito_dir = cwd.join(".cogito");
    std::fs::create_dir_all(&cogito_dir)
        .with_context(|| format!("Failed to create {}", cogito_dir.display()))?;
    Cogito::with_project_sqlite(&cogito_dir).with_context(|| {
        format!(
            "Failed to open SQLite at {}",
            cogito_dir.join("state.db").display()
        )
    })
}

/// Load or create a project for the current directory, registering it in SQLite
fn load_or_create_project(cogito: &Cogito, cwd: &Path) -> Result<Project> {
    let project = cogito
        .init_project(cwd.to_path_buf())
        .context("Failed to load/create project")?;

    // Register in SQLite so foreign keys resolve
    if let Some(sqlite) = cogito.sqlite() {
        let _ = cogito_core::storage::sqlite::upsert_project(
            &sqlite.lock(),
            project.id,
            &project.path,
            &project.name,
            &project.language.to_string(),
        );
    }

    Ok(project)
}

// ProjectId is just a type alias for Uuid
type _ProjectId = Uuid;
