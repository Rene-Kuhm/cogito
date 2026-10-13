//! MCP (Model Context Protocol) server for Cogito.
//!
//! Runs over stdio so any MCP-capable agent (Claude Code, Cursor, Codex,
//! opencode, Gemini CLI, …) can use Cogito as a tool provider.
//!
//! Each tool takes an explicit `path` argument. The server opens that
//! project's `.cogito/state.db` per call, so one process can serve several
//! projects without mixing their data.

use anyhow::Result;
use cogito_harness_core::{Cogito, CompressionOptions, Project};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};
use std::path::PathBuf;
use uuid::Uuid;

/// Server name advertised to MCP clients.
pub const SERVER_NAME: &str = "cogito";

// ============================================================
// Requests
// ============================================================

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ScanProjectRequest {
    /// Absolute path to the project directory
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CompressContextRequest {
    /// Content to compress
    pub content: String,
    /// Maximum tokens in the compressed output
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// Language hint for AST-aware compression (e.g. "rust", "typescript", "go")
    #[serde(default)]
    pub language: Option<String>,
}

fn default_max_tokens() -> usize {
    8000
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchKnowledgeRequest {
    /// Project directory path
    pub path: String,
    /// Search query
    pub query: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddDecisionRequest {
    /// Project directory path
    pub path: String,
    /// Decision title
    pub title: String,
    /// Why this decision was made
    pub why: String,
    /// Alternative options considered
    #[serde(default)]
    pub alternatives: Vec<String>,
    /// Affected paths
    #[serde(default)]
    pub paths: Vec<String>,
    /// Tags for categorization
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddTagRequest {
    /// Project directory path
    pub path: String,
    /// Tag name (e.g. "auth", "billing", "api")
    pub tag: String,
    /// Paths to tag
    pub paths: Vec<String>,
}

// ============================================================
// Server
// ============================================================

#[derive(Clone)]
pub struct CogitoServer {
    #[allow(dead_code)] // read by the #[tool_handler] macro expansion
    tool_router: ToolRouter<CogitoServer>,
}

impl Default for CogitoServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Open the project rooted at `path` and make sure it is registered.
///
/// This is the fix for the multi-project bug: the connection is derived from
/// the caller-supplied path, never from the process working directory.
fn open_project(path: &str) -> Result<(Cogito, Project), String> {
    let root = PathBuf::from(path);
    let cogito = Cogito::for_project(&root)
        .map_err(|e| format!("Failed to open project at {}: {e}", root.display()))?;
    let project = cogito
        .load_project(root.clone())
        .map_err(|e| format!("Failed to load project at {}: {e}", root.display()))?;
    Ok((cogito, project))
}

#[tool_router]
impl CogitoServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    /// Generate a code fingerprint for a project.
    #[tool(
        description = "Generate a code fingerprint (entry points, data flows, hot/cold zones) for a project. Returns a summary of the project structure and its most changed files according to git history, and stores the result for the project."
    )]
    async fn scan_project(
        &self,
        Parameters(req): Parameters<ScanProjectRequest>,
    ) -> Result<String, String> {
        let (cogito, project) = open_project(&req.path)?;

        let fingerprint = cogito
            .generate_fingerprint(&project)
            .await
            .map_err(|e| format!("Failed to generate fingerprint: {e}"))?;

        let mut out = format!(
            "Project: {} ({})\n\
             Entry points: {}\n\
             Data flows: {}\n\
             Hot zones: {}\n\
             Cold zones: {}\n\
             Frozen zones: {}\n\
             Generated: {}",
            project.name,
            project.language,
            fingerprint.entry_points.len(),
            fingerprint.data_flows.len(),
            fingerprint.hot_zones.len(),
            fingerprint.cold_zones.len(),
            fingerprint.frozen_zones.len(),
            fingerprint.generated_at,
        );

        if !fingerprint.hot_zones.is_empty() {
            out.push_str("\n\nMost active files:\n");
            for zone in fingerprint.hot_zones.iter().take(10) {
                let rel = zone.path.strip_prefix(&project.path).unwrap_or(&zone.path);
                out.push_str(&format!(
                    "  • {} ({} changes)\n",
                    rel.display(),
                    zone.change_count
                ));
            }
        }

        Ok(out)
    }

    /// Compress context to save tokens.
    #[tool(
        description = "Compress code/text context while preserving meaning. When a `language` is given and supported (rust, typescript, javascript, python, go, csharp, java, ruby, php, kotlin, swift), function bodies are collapsed to their signatures (AST-aware). Otherwise comments and whitespace are stripped."
    )]
    async fn compress_context(
        &self,
        Parameters(req): Parameters<CompressContextRequest>,
    ) -> Result<String, String> {
        let options = CompressionOptions {
            max_tokens: req.max_tokens,
            preserve_imports: true,
            preserve_types: true,
            hot_zones_priority: true,
        };

        let language = req
            .language
            .as_deref()
            .and_then(cogito_harness_core::compressor::parse_language);

        let result = cogito_harness_core::compressor::compress_with_language(
            &req.content,
            options,
            language,
        )
        .map_err(|e| format!("Failed to compress: {e}"))?;

        Ok(format!(
            "Original tokens: {}\n\
             Compressed tokens: {}\n\
             Savings: {:.1}%\n\n\
             --- Compressed Content ---\n{}",
            result.original_tokens,
            result.compressed_tokens,
            result.savings_percent,
            result.content
        ))
    }

    /// Search project knowledge (tags + decisions).
    #[tool(
        description = "Search project knowledge base: semantic tags and architectural decisions. Returns matches from both knowledge sources. Uses SQLite FTS5 with porter stemming, so 'scalable' also matches 'scalability'."
    )]
    async fn search_knowledge(
        &self,
        Parameters(req): Parameters<SearchKnowledgeRequest>,
    ) -> Result<String, String> {
        let (cogito, project) = open_project(&req.path)?;

        let tags = cogito
            .search_tags(project.id, &req.query)
            .map_err(|e| format!("Failed to search tags: {e}"))?;
        let decisions = cogito
            .search_decisions(project.id, &req.query)
            .map_err(|e| format!("Failed to search decisions: {e}"))?;

        let mut out = format!("Search: {}\n\n", req.query);
        out.push_str(&format!("Tags ({}):\n", tags.len()));
        for tag in &tags {
            out.push_str(&format!(
                "  • {} ({}) - {} paths\n",
                tag.tag,
                tag.domain,
                tag.paths.len()
            ));
        }
        out.push_str(&format!("\nDecisions ({}):\n", decisions.len()));
        for d in &decisions {
            out.push_str(&format!("  • {} - {}\n", d.title, d.why));
        }
        Ok(out)
    }

    /// Record an architectural decision.
    #[tool(
        description = "Record an architectural decision with title, rationale, alternatives, and tags. Helps the AI agent understand WHY past decisions were made."
    )]
    async fn add_decision(
        &self,
        Parameters(req): Parameters<AddDecisionRequest>,
    ) -> Result<String, String> {
        let (cogito, project) = open_project(&req.path)?;

        let mut builder = cogito_harness_core::decision::builder(project.id)
            .title(&req.title)
            .why(&req.why)
            .author(&whoami::username());
        for alt in &req.alternatives {
            builder = builder.alternative(alt);
        }
        for tag in &req.tags {
            builder = builder.tag(tag);
        }
        for p in &req.paths {
            builder = builder.path(PathBuf::from(p));
        }

        let added = cogito
            .add_decision(builder.build())
            .map_err(|e| format!("Failed to add decision: {e}"))?;

        Ok(format!(
            "Decision added\n  ID: {}\n  Title: {}\n  Why: {}\n  Project: {}",
            added.id, added.title, added.why, project.name
        ))
    }

    /// Tag project paths.
    #[tool(
        description = "Add a semantic tag (e.g. 'auth', 'billing') to project paths. Tags are searchable and help the AI agent find relevant code quickly."
    )]
    async fn add_tag(&self, Parameters(req): Parameters<AddTagRequest>) -> Result<String, String> {
        let (cogito, project) = open_project(&req.path)?;

        let tag = cogito_harness_core::types::SemanticTag {
            id: Uuid::new_v4(),
            project_id: project.id,
            tag: req.tag.clone(),
            domain: "manual".to_string(),
            paths: req.paths.iter().map(PathBuf::from).collect(),
            confidence: 1.0,
            created_at: chrono::Utc::now(),
        };

        let added = cogito
            .add_tag(tag)
            .map_err(|e| format!("Failed to add tag: {e}"))?;

        Ok(format!(
            "Tag added\n  Tag: {}\n  Domain: {}\n  Paths: {}\n  Project: {}",
            added.tag,
            added.domain,
            added.paths.len(),
            project.name
        ))
    }

    /// Project storage statistics.
    #[tool(
        description = "Report how much knowledge Cogito has stored for a project: number of tags and decisions, and when the project was last scanned (with its hot zone count)."
    )]
    async fn project_stats(
        &self,
        Parameters(req): Parameters<ScanProjectRequest>,
    ) -> Result<String, String> {
        let (cogito, project) = open_project(&req.path)?;
        let stats = cogito
            .stats(project.id)
            .map_err(|e| format!("Failed to read stats: {e}"))?;

        let fingerprint = match &stats.fingerprint_generated_at {
            Some(at) => format!("{} hot zones, scanned {at}", stats.hot_zones_count),
            None => "not scanned yet (use scan_project)".to_string(),
        };

        Ok(format!(
            "Project: {} ({})\n\
             Storage: {}\n\
             Tags: {}\n\
             Decisions: {}\n\
             Fingerprint: {}",
            project.name,
            project.language,
            cogito.cogito_dir().join("state.db").display(),
            stats.tags_count,
            stats.decisions_count,
            fingerprint,
        ))
    }
}

#[tool_handler]
impl ServerHandler for CogitoServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        rmcp::model::InitializeResult::new(
            ServerCapabilities::builder().enable_tools().build(),
        )
        // rmcp's `from_build_env` would report the crate name as "rmcp";
        // identify as cogito so clients show the right server.
        .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
        .with_instructions(
            "Cogito is a context harness for AI coding agents. It stores, per project: \
             a code fingerprint (entry points, data flows, hot/cold zones), semantic tags, \
             and architectural decisions. \
             Tools: scan_project, compress_context, search_knowledge, add_decision, add_tag, project_stats. \
             Every tool takes an explicit `path` to the project root; state lives in `<path>/.cogito/state.db`."
                .to_string(),
        )
    }
}

/// Run the MCP server over stdio until the client disconnects.
pub async fn serve_stdio() -> Result<()> {
    let service = CogitoServer::new()
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("MCP serving error: {e:?}"))?;
    service.waiting().await?;
    Ok(())
}
