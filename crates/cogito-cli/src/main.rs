//! Cogito CLI
//!
//! Intelligent context harness for AI coding agents.

mod commands;
mod mcp;

use anyhow::Result;
use clap::{CommandFactory, Parser};
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
    Scan {
        /// Watch mode for live updates
        #[arg(long)]
        watch: bool,
    },

    /// Show current fingerprint
    Fingerprint {
        /// Show detailed fingerprint
        #[arg(short, long)]
        verbose: bool,
    },

    /// Compress context
    Compress {
        /// Input file (reads from stdin if not provided)
        #[arg(default_value = "-")]
        file: String,

        /// Maximum tokens in output
        #[arg(long, default_value_t = 8000)]
        max_tokens: usize,

        /// Preserve imports
        #[arg(long, default_value_t = true)]
        preserve_imports: bool,

        /// Preserve types
        #[arg(long, default_value_t = true)]
        preserve_types: bool,
    },

    /// Query knowledge base
    Query {
        /// Search query
        query: String,
    },

    /// Manage decisions
    Decision {
        #[command(subcommand)]
        subcommand: Option<DecisionCommands>,
    },

    /// Manage semantic tags
    Tag {
        #[command(subcommand)]
        subcommand: Option<TagCommands>,
    },

    /// Manage projects
    Project {
        #[command(subcommand)]
        subcommand: Option<ProjectCommands>,
    },

    /// Run the MCP server over stdio (for AI agents)
    Mcp,

    /// Start server mode
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value_t = 8080)]
        port: u16,

        /// Start as MCP server
        #[arg(long)]
        mcp: bool,
    },

    /// Show statistics
    Stats,

    /// Diagnose configuration
    Doctor,

    /// Export project data
    Export {
        /// Output file (stdout if not provided)
        #[arg(short, long)]
        output: Option<String>,

        /// Export format
        #[arg(short, long, default_value = "json")]
        format: String,
    },
}

#[derive(Parser, Debug)]
enum DecisionCommands {
    /// Add a new decision
    Add {
        /// Decision title
        #[arg(short, long)]
        title: Option<String>,

        /// Why this decision was made
        #[arg(short, long)]
        why: Option<String>,

        /// Alternative options considered
        #[arg(short, long)]
        alternative: Vec<String>,

        /// Affected paths
        #[arg(short, long)]
        paths: Vec<String>,

        /// Tags
        #[arg(short = 'T', long)]
        tags: Vec<String>,
    },

    /// List all decisions
    List,

    /// Search decisions
    Search {
        /// Search query
        query: String,
    },

    /// Show decision details
    Show {
        /// Decision ID
        id: String,
    },
}

#[derive(Parser, Debug)]
enum TagCommands {
    /// Add a semantic tag
    Add {
        /// Tag name
        tag: String,

        /// Paths to tag
        paths: Vec<String>,
    },

    /// List all tags
    List,

    /// Search tags
    Search {
        /// Search query
        query: String,
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

    tracing_subscriber::registry()
        .with(fmt::layer().with_target(true))
        .with(filter)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    setup_logging(args.verbose, args.quiet);

    match args.command {
        Some(Commands::Init) => commands::init().await?,
        Some(Commands::Scan { watch }) => commands::scan(watch).await?,
        Some(Commands::Fingerprint { verbose }) => commands::fingerprint(verbose).await?,
        Some(Commands::Compress {
            file,
            max_tokens,
            preserve_imports,
            preserve_types,
        }) => {
            commands::compress(
                &file,
                max_tokens,
                preserve_imports,
                preserve_types,
                args.json,
            )
            .await?;
        }
        Some(Commands::Query { query }) => commands::query(&query, args.json).await?,
        Some(Commands::Decision { subcommand }) => {
            commands::decision(subcommand).await?;
        }
        Some(Commands::Tag { subcommand }) => {
            commands::tag(subcommand).await?;
        }
        Some(Commands::Project { subcommand }) => {
            commands::project(subcommand).await?;
        }
        Some(Commands::Mcp) => mcp::serve_stdio().await?,
        Some(Commands::Serve { port, mcp }) => commands::serve(port, mcp).await?,
        Some(Commands::Stats) => commands::stats(args.json).await?,
        Some(Commands::Doctor) => commands::doctor().await?,
        Some(Commands::Export { output, format }) => {
            commands::export(output.as_deref(), &format).await?
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
