//! CLI Commands
//!
//! Connects to the cogito-harness-core engine and provides user-facing operations.

use anyhow::{Context, Result};
use cogito_harness_core::{Cogito, Project};
use std::path::Path;

use crate::ProjectCommands;

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
        let _ = cogito_harness_core::storage::sqlite::upsert_project(
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
pub async fn scan() -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;

    println!("Scanning: {} ({})", project.name, project.language);

    // Sync the index and report what changed
    if let Some(conn) = cogito.sqlite() {
        let conn_guard = conn.lock();
        let report =
            cogito_harness_core::index::sync(&conn_guard, project.id, &project.path, false)?;
        println!();
        println!("📁 Index: {} files scanned", report.scanned);
        if report.added > 0 {
            println!("   + {} added", report.added);
        }
        if report.modified > 0 {
            println!("   ~ {} modified", report.modified);
        }
        if report.removed > 0 {
            println!("   - {} removed", report.removed);
        }
        if report.reparsed > 0 {
            println!("   🔄 {} reparsed", report.reparsed);
        }
        if report.unchanged > 0 {
            println!("   ✓ {} unchanged", report.unchanged);
        }
    }
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
        scan().await
    } else {
        println!("Use 'cogito scan' to regenerate the fingerprint.");
        Ok(())
    }
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
// Stats / Doctor
// ============================================================

/// Print the project map, scanning only if the stored one is stale.
pub async fn map(refresh: bool, card: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;
    let (fingerprint, _) = cogito.fingerprint(&project, refresh).await?;
    let map = cogito_harness_core::map::build(&project.path, &project.name, &fingerprint);
    if card {
        print!("{}", cogito_harness_core::map::render_card(&map));
    } else {
        print!("{}", cogito_harness_core::map::render(&map));
    }
    Ok(())
}

/// Print `file`'s declarations or sections with line ranges.
pub async fn outline(file: &str) -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;
    let _ = cogito.sync_index(&project).await?;

    // Need a fingerprint for find_file. Try stored first, then fall back to scan.
    let stored = cogito.stored_fingerprint(project.id)?;
    let fingerprint = match stored.as_ref() {
        Some(fp) => fp.clone(),
        None => {
            let (fp, _) = cogito.fingerprint(&project, false).await?;
            fp
        }
    };
    let file = cogito_harness_core::map::find_file(&project.path, &fingerprint, file)
        .map_err(|e| anyhow::anyhow!(e))?;

    // Try index first.
    #[allow(clippy::collapsible_if)]
    if let Some(conn) = cogito.connection() {
        if let Ok(decls) = cogito_harness_core::index::outline(&conn, project.id, &file) {
            if !decls.is_empty() {
                let importer_count =
                    cogito_harness_core::index::importers(&conn, project.id, &file)
                        .map(|i| i.len())
                        .unwrap_or(0);
                let text = cogito_harness_core::map::render_outline(
                    &project.path,
                    stored.as_ref(),
                    &file,
                    &decls.into_iter().map(Into::into).collect::<Vec<_>>(),
                    importer_count,
                );
                print!("{text}");
                return Ok(());
            }
        }
    }

    // Index had no declarations: read from disk.
    let content = std::fs::read_to_string(project.path.join(&file))
        .map_err(|e| anyhow::anyhow!("Cannot read {}: {e}", file.display()))?;
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    let decls = cogito_harness_core::map::outline::declarations(&content, ext);
    let text =
        cogito_harness_core::map::render_outline(&project.path, stored.as_ref(), &file, &decls, 0);
    print!("{text}");
    Ok(())
}

/// Print symbols, doc sections, files and modules matching `query`.
pub async fn search(query: &str) -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;
    let _ = cogito.sync_index(&project).await?;

    let stored = cogito.stored_fingerprint(project.id)?;

    let hits = if let Some(conn) = cogito.connection() {
        cogito_harness_core::index::search(&conn, project.id, query, 12)?
    } else {
        vec![]
    };

    let query_lower = query.to_lowercase();
    let files_first = hits.iter().any(|h| {
        h.kind != "symbol"
            && h.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_lowercase() == query_lower)
                .unwrap_or(false)
    });

    let modules: Vec<_> = stored
        .as_ref()
        .map(|fp| fp.modules.clone())
        .unwrap_or_default();

    print!(
        "{}",
        cogito_harness_core::map::render_search(&hits, &modules, query, files_first)
    );
    Ok(())
}

/// Print what `file` imports and what imports it.
pub async fn related(file: &str) -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;
    let _ = cogito.sync_index(&project).await?;

    // Need a fingerprint for find_file. Try stored first, then fall back to scan.
    let stored = cogito.stored_fingerprint(project.id)?;
    let fingerprint = match stored.as_ref() {
        Some(fp) => fp.clone(),
        None => {
            let (fp, _) = cogito.fingerprint(&project, false).await?;
            fp
        }
    };
    let file = cogito_harness_core::map::find_file(&project.path, &fingerprint, file)
        .map_err(|e| anyhow::anyhow!(e))?;

    let (imports, importers, external) = if let Some(conn) = cogito.connection() {
        let imports: Vec<String> =
            cogito_harness_core::index::imports(&conn, project.id, &file).unwrap_or_default();
        let importers: Vec<String> =
            cogito_harness_core::index::importers(&conn, project.id, &file)
                .unwrap_or_default()
                .into_iter()
                .map(|p| p.display().to_string())
                .collect();
        let external: Vec<String> = stored
            .as_ref()
            .map(|fp| {
                fp.data_flows
                    .iter()
                    .filter(|flow| flow.from == file && flow.resolved.is_none())
                    .map(|flow| flow.to.display().to_string())
                    .collect()
            })
            .unwrap_or_default();
        (imports, importers, external)
    } else {
        (vec![], vec![], vec![])
    };

    print!(
        "{}",
        cogito_harness_core::map::render_related(
            &project.path,
            stored.as_ref(),
            &file,
            &imports,
            &importers,
            &external,
        )
    );
    Ok(())
}

pub async fn stats(json: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("Cannot read current directory")?;
    let cogito = open_cogito(&cwd)?;
    let project = load_or_create_project(&cogito, &cwd)?;
    let stored = cogito.stats(project.id)?;

    if json {
        let stats = serde_json::json!({
            "project": project.name,
            "has_fingerprint": stored.has_fingerprint,
            "fingerprint_generated_at": stored.fingerprint_generated_at,
        });
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("=== Cogito Statistics ===");
        println!("Project: {}", project.name);
        match &stored.fingerprint_generated_at {
            Some(at) => println!("Fingerprint: present, generated {at}"),
            None => println!("Fingerprint: not scanned yet (run `cogito scan`)"),
        }
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

    // The Pi package is optional and separate from the MCP entry, so it gets
    // its own line rather than making Gentle Shell look unconfigured.
    if crate::agents::Agent::GentleShell.agent_present(&home) {
        let declared = crate::agents::pi_package_declared(&home);
        if declared {
            let settings = crate::agents::Agent::GentleShell.config_path(&home);
            println!();
            // Declared is not the same as installable: Pi runs `npm install` for
            // every declared package, and a 404 there breaks package loading.
            if crate::agents::pi_package_check() == crate::agents::NpmCheck::NotFound {
                println!(
                    "  ! {} declared in {}",
                    crate::agents::PI_PACKAGE,
                    settings.display()
                );
                println!("    but it is not on npm, so Pi will fail to install it;");
                println!("    run `cogito setup` to remove the declaration");
            } else {
                println!(
                    "  ✓ {} declared in {}",
                    crate::agents::PI_PACKAGE,
                    settings.display()
                );
            }
        } else {
            let check = crate::agents::pi_package_check();
            println!();
            println!("  · {} not declared", crate::agents::PI_PACKAGE);
            println!(
                "    {}",
                match check {
                    crate::agents::NpmCheck::Published => {
                        "available on npm — re-run `cogito setup` to declare it"
                    }
                    crate::agents::NpmCheck::NotFound => {
                        "not on npm yet; the MCP tools work without it"
                    }
                    crate::agents::NpmCheck::Unavailable(_) => {
                        "npm unreachable; the MCP tools work without it"
                    }
                }
            );
        }
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
pub fn setup(agents: Vec<String>, all: bool, dry_run: bool, with_pi_package: bool) -> Result<()> {
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
        println!("         Install first (`cargo install --path crates/cogito-harness` or");
        println!("         install.sh) and re-run setup from the installed binary.");
    }
    println!();

    // Resolve the Pi package once, before writing anything.
    //
    // Pi runs `npm install` for every entry in `settings.json`, so declaring a
    // package the registry does not have makes the agent fail to start. The
    // package is therefore only declared when npm can actually install it.
    let needs_pi_package = selected.iter().any(|a| a.uses_pi_package());
    let pi_package = if needs_pi_package {
        Some(crate::agents::pi_package_check())
    } else {
        None
    };
    let pi_action = match (&pi_package, with_pi_package) {
        // An explicit request wins: the user knows it is available.
        (Some(_), true) => crate::agents::PiPackageAction::Declare,
        (Some(check), false) => match check {
            crate::agents::NpmCheck::Published => crate::agents::PiPackageAction::Declare,
            // Positively absent: also clear a declaration that would break Pi.
            crate::agents::NpmCheck::NotFound => crate::agents::PiPackageAction::Remove,
            // Could not tell: leave whatever the user has alone.
            crate::agents::NpmCheck::Unavailable(_) => crate::agents::PiPackageAction::Leave,
        },
        (None, _) => crate::agents::PiPackageAction::Leave,
    };

    if let Some(check) = &pi_package {
        println!("Pi package  {}", crate::agents::PI_PACKAGE);
        match check {
            crate::agents::NpmCheck::Published => {
                println!("            found on npm; it will be declared");
            }
            crate::agents::NpmCheck::NotFound => {
                println!("            not on npm yet; only the MCP half will be configured");
                println!("            (declaring it now would make Pi fail to install packages)");
                println!("            re-run setup after it is published, or force it with");
                println!("            --with-pi-package if you know it is available");
            }
            crate::agents::NpmCheck::Unavailable(reason) => {
                println!("            could not check npm ({reason})");
                println!(
                    "            leaving its declaration as it is; the MCP half is configured either way"
                );
                println!("            use --with-pi-package to declare it regardless");
            }
        }
        println!();
    }

    for agent in selected {
        if agent.uses_pi_package() && pi_action == crate::agents::PiPackageAction::Declare {
            println!("  {}:", agent.display());
            println!(
                "    declares the npm package `{}` in {}",
                crate::agents::PI_PACKAGE,
                agent.config_path(&home).display()
            );
        } else if agent.uses_pi_package() {
            println!("  {}:", agent.display());
            println!(
                "    MCP only; `{}` is not declared",
                crate::agents::PI_PACKAGE
            );
        }

        // Project root for AGENTS.md block: the current working directory.
        // Agents that read AGENTS.md (Codex, opencode) get a block here.
        let project_root = std::env::current_dir()?;

        if dry_run {
            let status = crate::agents::status(agent, &home, &binary)?;
            let agents_action = match status {
                crate::agents::Status::Configured => "stay as is",
                crate::agents::Status::Outdated => "update the entry",
                _ => "write",
            };

            if crate::agents::reads_agents_md(agent) {
                // Print the AGENTS.md action first (from the function), then our summary line.
                let _ = crate::agents::install_agents_md_block(&project_root, true);
                println!(
                    "  {} [{}] -> {}, AGENTS.md block would be written",
                    agent.display(),
                    status.label(),
                    agents_action
                );
            } else {
                println!(
                    "  {} [{}] -> would {}",
                    agent.display(),
                    status.label(),
                    agents_action
                );
            }
            continue;
        }

        match crate::agents::install(agent, &home, &binary, pi_action) {
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

        // Install the AGENTS.md block for agents that read it.
        if crate::agents::reads_agents_md(agent) {
            if dry_run {
                let _ = crate::agents::install_agents_md_block(&project_root, true);
            } else {
                match crate::agents::install_agents_md_block(&project_root, false) {
                    Ok(crate::agents::Change::Written { .. }) => {
                        println!("    AGENTS.md: block installed");
                    }
                    Ok(crate::agents::Change::AlreadyCorrect) => {
                        println!("    AGENTS.md: block already present");
                    }
                    Err(e) => {
                        println!("    AGENTS.md: failed: {e}");
                    }
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
    // Project root for AGENTS.md block: the current working directory.
    let project_root = std::env::current_dir()?;

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

        // Remove the AGENTS.md block for agents that read it.
        if crate::agents::reads_agents_md(agent) {
            match crate::agents::remove_agents_md_block(&project_root) {
                Ok(crate::agents::Change::Written { .. }) => {
                    println!("    AGENTS.md: block removed");
                }
                Ok(crate::agents::Change::AlreadyCorrect) => {
                    // No block to remove; nothing to report.
                }
                Err(e) => {
                    println!("    AGENTS.md: failed to remove block: {e}");
                }
            }
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

// ============================================================
// Helpers
// ============================================================

/// Open a Cogito instance backed by SQLite at `<cwd>/.cogito/state.db`.
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
        let _ = cogito_harness_core::storage::sqlite::upsert_project(
            &sqlite.lock(),
            project.id,
            &project.path,
            &project.name,
            &project.language.to_string(),
        );
    }

    Ok(project)
}
