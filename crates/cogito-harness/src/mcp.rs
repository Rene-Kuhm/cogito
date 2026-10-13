//! MCP (Model Context Protocol) server for Cogito.
//!
//! Runs over stdio so any MCP-capable agent (Claude Code, Cursor, Codex,
//! opencode, Gemini CLI, …) can use Cogito as a tool provider.
//!
//! The tools give an agent the project's layout so it does not have to read
//! the code to find its way: `project_map`, `related_files`, `search_code`,
//! and `add_tag` to label code locations. Memory (decisions, session notes)
//! is deliberately out of scope; that belongs to the agent's memory tool.
//!
//! Each tool takes an explicit `path` argument. The server opens that
//! project's `.cogito/state.db` per call, so one process can serve several
//! projects without mixing their data.

use anyhow::Result;
use cogito_harness_core::{Cogito, Fingerprint, Project};
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

/// What the client tells the model about this server.
const INSTRUCTIONS: &str = "Cogito keeps a map of the project's code so you can find your way \
without reading it. When you start working in a repository, call project_map before listing \
directories or opening files to orient yourself; use related_files to see what a file imports \
and what imports it, and search_code to locate code by path, module or tag. The map is \
refreshed automatically when HEAD changes; pass refresh=true after large uncommitted changes. \
Cogito is not a memory: it stores no decisions or session notes, keep those in your memory \
tool. Every tool takes `path`, the absolute project root.";

// ============================================================
// Requests
// ============================================================

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectMapRequest {
    /// Absolute path to the project root
    pub path: String,
    /// Rescan even if the stored map is current (after large uncommitted changes)
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RelatedFilesRequest {
    /// Absolute path to the project root
    pub path: String,
    /// File to inspect: a path relative to the root, or a unique suffix such as `auth/server.ts`
    pub file: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchCodeRequest {
    /// Absolute path to the project root
    pub path: String,
    /// Words to look for in file paths, module names and tags, e.g. `auth session`
    pub query: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddTagRequest {
    /// Absolute path to the project root
    pub path: String,
    /// Tag name, e.g. "auth", "billing"
    pub tag: String,
    /// Project paths the tag points at
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
        .map_err(|e| format!("Failed to open project at {}: {e:#}", root.display()))?;
    let project = cogito
        .load_project(root.clone())
        .map_err(|e| format!("Failed to load project at {}: {e:#}", root.display()))?;
    Ok((cogito, project))
}

/// The project and its fingerprint, scanning only if the stored one is stale.
async fn mapped_project(
    path: &str,
    refresh: bool,
) -> Result<(Cogito, Project, Fingerprint), String> {
    let (cogito, project) = open_project(path)?;
    let (fingerprint, _) = cogito
        .fingerprint(&project, refresh)
        .await
        .map_err(|e| format!("Failed to scan {}: {e}", project.path.display()))?;
    Ok((cogito, project, fingerprint))
}

#[tool_router]
impl CogitoServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Map of the project's code: modules with their entry point, size, most changed files and which modules they use, plus the most changed files overall. Read this before exploring the repository."
    )]
    async fn project_map(
        &self,
        Parameters(req): Parameters<ProjectMapRequest>,
    ) -> Result<String, String> {
        let (_, project, fingerprint) = mapped_project(&req.path, req.refresh).await?;
        let map = cogito_harness_core::map::build(&project.path, &project.name, &fingerprint);
        Ok(cogito_harness_core::map::render(&map))
    }

    #[tool(
        description = "A file's neighbourhood: the project files it imports, the files that import it, and the external packages it uses."
    )]
    async fn related_files(
        &self,
        Parameters(req): Parameters<RelatedFilesRequest>,
    ) -> Result<String, String> {
        let (_, project, fingerprint) = mapped_project(&req.path, false).await?;
        let file = cogito_harness_core::map::find_file(&project.path, &fingerprint, &req.file)?;
        Ok(cogito_harness_core::map::render_related(
            &project.path,
            &fingerprint,
            &file,
        ))
    }

    #[tool(
        description = "Find code by words: matching file paths (most changed first), modules and tags."
    )]
    async fn search_code(
        &self,
        Parameters(req): Parameters<SearchCodeRequest>,
    ) -> Result<String, String> {
        let (cogito, project, fingerprint) = mapped_project(&req.path, false).await?;
        let (files, modules) =
            cogito_harness_core::map::search_paths(&project.path, &fingerprint, &req.query);
        let tags = cogito
            .search_tags(project.id, &req.query)
            .map_err(|e| format!("Failed to search tags: {e}"))?;

        if files.is_empty() && modules.is_empty() && tags.is_empty() {
            return Ok(format!("Nothing matches \"{}\".", req.query));
        }
        let mut out = String::new();
        if !tags.is_empty() {
            out.push_str("Tags:\n");
            for tag in &tags {
                let paths: Vec<String> =
                    tag.paths.iter().map(|p| p.display().to_string()).collect();
                out.push_str(&format!("  {}: {}\n", tag.tag, paths.join(", ")));
            }
        }
        if !modules.is_empty() {
            out.push_str(&format!("Modules:\n  {}\n", modules.join("\n  ")));
        }
        if !files.is_empty() {
            out.push_str(&format!("Files:\n  {}\n", files.join("\n  ")));
        }
        Ok(out)
    }

    #[tool(
        description = "Label project paths with a tag (e.g. 'auth' → apps/web/lib/auth) so search_code finds them by that word later. For code locations only; not for notes or decisions."
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
            "Tagged {} path(s) as '{}' in {}",
            added.paths.len(),
            added.tag,
            project.name
        ))
    }
}

#[tool_handler]
impl ServerHandler for CogitoServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        rmcp::model::InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            // rmcp's `from_build_env` would report the crate name as "rmcp";
            // identify as cogito so clients show the right server.
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS.to_string())
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
