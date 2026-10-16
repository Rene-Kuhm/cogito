//! Cogito CLI
//!
//! Intelligent context harness for AI coding agents.

mod agents;
mod claude_hook;
mod claude_post_tool_use;
mod commands;
mod mcp;
mod tokenizer;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Cogito - Intelligent context optimization for AI coding agents
#[derive(Parser, Debug)]
#[command(
    name = "cogito",
    version,
    author,
    about = "Intelligent context harness for AI coding agents",
    long_about = None
)]
struct Args {
    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Quiet mode (only errors)
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Format output as JSON
    #[arg(short, long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Parser, Debug)]
enum Commands {
    /// Initialize Cogito in the current directory
    Init,

    /// Scan project and generate fingerprint
    Scan,

    /// Show current fingerprint
    Fingerprint {
        /// Show detailed fingerprint
        #[arg(short, long)]
        verbose: bool,
    },

    /// Manage projects
    Project {
        #[command(subcommand)]
        subcommand: Option<ProjectCommands>,
    },

    /// Run the MCP server over stdio (for AI agents)
    Mcp,

    /// Install Cogito into one or more AI agents
    Setup {
        /// Agents to configure (omit for an interactive menu)
        agents: Vec<String>,
        /// Configure every detected agent without asking
        #[arg(long)]
        all: bool,
        /// Assume yes: configure every detected agent without prompting
        /// (same as --all; accepted so `setup -y` behaves as expected)
        #[arg(short = 'y', long)]
        yes: bool,
        /// Show what would change without writing anything
        #[arg(long)]
        dry_run: bool,
        /// Declare Pi's npm package even if the registry check cannot confirm
        /// it is installable. Only use this when you know it is published:
        /// Pi runs `npm install` for every declared package, and a 404 there
        /// stops the agent from starting.
        #[arg(long)]
        with_pi_package: bool,
    },

    /// Remove Cogito from one or more AI agents
    Uninstall {
        /// Agents to clean up (omit to remove from every configured agent)
        agents: Vec<String>,
        /// Skip the confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Show statistics
    Stats,

    /// Run an agent hook (called by the agent, not by hand)
    #[command(hide = true)]
    Hook {
        /// Which hook: `claude-pre-tool-use` or `claude-post-tool-use`
        name: String,
    },

    /// Decide one tool call for any agent: a call as JSON on stdin, the decision
    /// as JSON on stdout. Agents without Claude Code's hook protocol call this
    /// from their own extension.
    #[command(hide = true)]
    Guard,

    /// Print the project map an agent gets from the `project_map` tool
    Map {
        /// Rescan even if the stored map is current
        #[arg(long)]
        refresh: bool,
        /// Print the short project card agents get in their system prompt
        #[arg(long)]
        card: bool,
    },

    /// Show a file's declarations or sections with line ranges (the `file_outline` tool)
    Outline {
        /// File path, relative to the project root, or a unique suffix
        file: String,
    },

    /// Find symbols, doc sections, files and modules by words (the `search_code` tool)
    Search {
        /// Words to look for
        query: String,
    },

    /// Show a file's imports and importers (the `related_files` tool)
    Related {
        /// File path, relative to the project root, or a unique suffix
        file: String,
    },

    /// Diagnose configuration
    Doctor,

    /// Measure token savings when using Cogito vs. a baseline explorer.
    Bench {
        /// Local repository to benchmark. When omitted, uses the pinned manifest
        /// at benchmarks/repos.json and clones repositories into .cogito/bench-cache/.
        #[arg(short = 'r', long)]
        repo: Option<String>,
        /// Comma-separated search terms (e.g. "search_code,project_map").
        /// When omitted, uses the terms from the manifest for that repository.
        #[arg(short = 't', long, value_delimiter = ',', num_args = 1..)]
        terms: Vec<String>,
        /// Maximum matched files to read per term (default 5).
        #[arg(short, long, default_value = "5")]
        limit: usize,
        /// Print the three-section measurement report: quality (recall@budget),
        /// latency and cost, each on its own and never combined into a score.
        /// Off by default, so the output without it is unchanged.
        #[arg(long)]
        report: bool,
        /// Root that holds one <timestamp>/ artifact per run. Defaults to
        /// ~/.cache/cogito/bench-runs, outside any measured worktree.
        #[arg(long)]
        out_dir: Option<String>,
        /// Repeat the measurement N times. Every repetition is kept in the
        /// run artifact; quality and cost are deterministic, latency is not.
        #[arg(long, default_value = "1")]
        repeat: usize,
        /// Labelled gold set used for recall@budget. Defaults to
        /// benchmarks/context_pack_gold.json next to the manifest.
        #[arg(long)]
        gold: Option<String>,
    },
}

#[derive(Parser, Debug)]
enum ProjectCommands {
    /// List all projects
    List,

    /// Delete a project
    Delete {
        /// Project ID
        id: String,
    },
}

fn setup_logging(verbose: bool, quiet: bool) {
    let filter = if quiet {
        EnvFilter::new("error")
    } else if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    // Logs go to stderr. stdout carries program output: JSON for `--json`, and
    // for `cogito mcp` the JSON-RPC stream itself, where any other line is a
    // protocol error for the client.
    let stderr_is_terminal = std::io::IsTerminal::is_terminal(&std::io::stderr());
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_target(true)
                .with_writer(std::io::stderr)
                .with_ansi(stderr_is_terminal),
        )
        .with(filter)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    // A hook's stderr is shown to the model, so hooks log nothing.
    let quiet = args.quiet
        || matches!(
            args.command,
            Some(Commands::Hook { .. }) | Some(Commands::Guard)
        );
    setup_logging(args.verbose, quiet);

    match args.command {
        Some(Commands::Init) => commands::init().await?,
        Some(Commands::Hook { name }) => match name.as_str() {
            "claude-pre-tool-use" => claude_hook::run().await,
            "claude-post-tool-use" => claude_post_tool_use::run().await,
            // An unknown hook must never block the agent.
            _ => std::process::exit(0),
        },
        Some(Commands::Guard) => claude_hook::run_guard().await,
        Some(Commands::Scan) => commands::scan().await?,
        Some(Commands::Fingerprint { verbose }) => commands::fingerprint(verbose).await?,
        Some(Commands::Project { subcommand }) => {
            commands::project(subcommand).await?;
        }
        Some(Commands::Mcp) => mcp::serve_stdio().await?,
        Some(Commands::Setup {
            agents,
            all,
            yes,
            dry_run,
            with_pi_package,
        }) => commands::setup(agents, all || yes, dry_run, with_pi_package)?,
        Some(Commands::Uninstall { agents, yes }) => commands::uninstall(agents, yes)?,
        Some(Commands::Stats) => commands::stats(args.json).await?,
        Some(Commands::Map { refresh, card }) => commands::map(refresh, card).await?,
        Some(Commands::Related { file }) => commands::related(&file).await?,
        Some(Commands::Outline { file }) => commands::outline(&file).await?,
        Some(Commands::Search { query }) => commands::search(&query).await?,
        Some(Commands::Doctor) => commands::doctor().await?,
        Some(Commands::Bench {
            repo,
            terms,
            limit,
            report,
            out_dir,
            repeat,
            gold,
        }) => {
            commands::bench(
                commands::bench::BenchOptions {
                    repo: repo.map(PathBuf::from),
                    terms,
                    limit,
                    report,
                    out_dir: out_dir.map(PathBuf::from),
                    repeat,
                    gold: gold.map(PathBuf::from),
                },
                args.json,
            )
            .await?;
        }
        None => {
            // No subcommand, show help
            let mut cmd = Args::command();
            cmd.print_help()?;
            println!();
        }
    }

    Ok(())
}
