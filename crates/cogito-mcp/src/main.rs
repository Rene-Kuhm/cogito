//! Cogito MCP (Model Context Protocol) Server
//!
//! Exposes cogito's context optimization capabilities as MCP tools
//! so any MCP-compatible agent (Claude Code, Cursor, etc.) can use them.

use anyhow::Result;
use cogito_core::{Cogito, CompressionOptions};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::ServerCapabilities,
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};
use std::path::PathBuf;
use tracing_subscriber::{self, EnvFilter};
use uuid::Uuid;

// ============================================================
// Request/Response Types
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
    /// Preserve import statements
    #[serde(default = "default_true")]
    pub preserve_imports: bool,
    /// Preserve type definitions
    #[serde(default = "default_true")]
    pub preserve_types: bool,
}

fn default_max_tokens() -> usize {
    8000
}
fn default_true() -> bool {
    true
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
// Cogito Server
// ============================================================

#[derive(Clone)]
pub struct CogitoServer {
    #[allow(dead_code)]
    cogito: Cogito,
    #[allow(dead_code)]
    tool_router: ToolRouter<CogitoServer>,
}

impl Default for CogitoServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router]
impl CogitoServer {
    pub fn new() -> Self {
        Self {
            cogito: Cogito::new(),
            tool_router: Self::tool_router(),
        }
    }

    /// Generate a code fingerprint for a project
    #[tool(
        description = "Generate a code fingerprint (entry points, data flows, hot/cold zones) for a project. Returns a summary of the project structure and zone analysis based on git history."
    )]
    async fn scan_project(
        &self,
        Parameters(req): Parameters<ScanProjectRequest>,
    ) -> Result<String, String> {
        let project = self
            .cogito
            .init_project(PathBuf::from(&req.path))
            .map_err(|e| format!("Failed to init project: {e}"))?;

        let fingerprint = self
            .cogito
            .generate_fingerprint(&project)
            .await
            .map_err(|e| format!("Failed to generate fingerprint: {e}"))?;

        Ok(format!(
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
        ))
    }

    /// Compress context to save tokens
    #[tool(
        description = "Compress code/text context while preserving meaning. Strips comments, compresses whitespace. Use this to reduce token usage before sending context to an AI model."
    )]
    async fn compress_context(
        &self,
        Parameters(req): Parameters<CompressContextRequest>,
    ) -> Result<String, String> {
        let options = CompressionOptions {
            max_tokens: req.max_tokens,
            preserve_imports: req.preserve_imports,
            preserve_types: req.preserve_types,
            hot_zones_priority: true,
        };

        let result = self
            .cogito
            .compress_context(&req.content, options)
            .await
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

    /// Search project knowledge (tags + decisions)
    #[tool(
        description = "Search project knowledge base: semantic tags and architectural decisions. Returns matches from both knowledge sources."
    )]
    async fn search_knowledge(
        &self,
        Parameters(req): Parameters<SearchKnowledgeRequest>,
    ) -> Result<String, String> {
        let project = self
            .cogito
            .init_project(PathBuf::from(&req.path))
            .map_err(|e| format!("Failed to init project: {e}"))?;

        let tags = self
            .cogito
            .search_tags(project.id, &req.query)
            .await
            .map_err(|e| format!("Failed to search tags: {e}"))?;
        let decisions = self
            .cogito
            .search_decisions(project.id, &req.query)
            .await
            .map_err(|e| format!("Failed to search decisions: {e}"))?;

        let mut output = format!("Search: {}\n\n", req.query);
        output.push_str(&format!("📌 Tags ({}):\n", tags.len()));
        for tag in &tags {
            output.push_str(&format!(
                "  • {} ({}) - {} paths\n",
                tag.tag,
                tag.domain,
                tag.paths.len()
            ));
        }
        output.push_str(&format!("\n📋 Decisions ({}):\n", decisions.len()));
        for d in &decisions {
            output.push_str(&format!("  • {} - {}\n", d.title, d.why));
        }
        Ok(output)
    }

    /// Add a decision to the project log
    #[tool(
        description = "Record an architectural decision with title, rationale, alternatives, and tags. Helps the AI agent understand WHY past decisions were made."
    )]
    async fn add_decision(
        &self,
        Parameters(req): Parameters<AddDecisionRequest>,
    ) -> Result<String, String> {
        let project = self
            .cogito
            .init_project(PathBuf::from(&req.path))
            .map_err(|e| format!("Failed to init project: {e}"))?;

        let mut builder = cogito_core::decision::builder(project.id)
            .title(&req.title)
            .why(&req.why)
            .author(&whoami::username());
        for alt in &req.alternatives {
            builder = builder.alternative(alt);
        }
        for tag in &req.tags {
            builder = builder.tag(tag);
        }
        for path_str in &req.paths {
            builder = builder.path(PathBuf::from(path_str));
        }
        let decision = builder.build();

        let added = self
            .cogito
            .add_decision(decision)
            .await
            .map_err(|e| format!("Failed to add decision: {e}"))?;

        Ok(format!(
            "✓ Decision added\n  ID: {}\n  Title: {}\n  Why: {}",
            added.id, added.title, added.why
        ))
    }

    /// Add a semantic tag to project paths
    #[tool(
        description = "Add a semantic tag (e.g. 'auth', 'billing') to project paths. Tags are searchable and help the AI agent find relevant code quickly."
    )]
    async fn add_tag(&self, Parameters(req): Parameters<AddTagRequest>) -> Result<String, String> {
        let project = self
            .cogito
            .init_project(PathBuf::from(&req.path))
            .map_err(|e| format!("Failed to init project: {e}"))?;

        let semantic_tag = cogito_core::types::SemanticTag {
            id: Uuid::new_v4(),
            project_id: project.id,
            tag: req.tag.clone(),
            domain: "manual".to_string(),
            paths: req.paths.iter().map(PathBuf::from).collect(),
            confidence: 1.0,
            created_at: chrono::Utc::now(),
        };

        let added = self
            .cogito
            .add_tag(semantic_tag)
            .await
            .map_err(|e| format!("Failed to add tag: {e}"))?;

        Ok(format!(
            "✓ Tag added\n  Tag: {}\n  Domain: {}\n  Paths: {}",
            added.tag,
            added.domain,
            added.paths.len()
        ))
    }
}

#[tool_handler]
impl ServerHandler for CogitoServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        rmcp::model::InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Cogito MCP server provides context optimization for AI coding agents. \
             Tools: scan_project (generate fingerprint), compress_context (reduce tokens), \
             search_knowledge (find tags/decisions), add_decision (record architecture decisions), \
             add_tag (categorize code). \
             All tools work with file-based persistence in .cogito/ directory.",
            )
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("Starting Cogito MCP server");

    let service = CogitoServer::new()
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("serving error: {:?}", e))?;

    service.waiting().await?;
    Ok(())
}
