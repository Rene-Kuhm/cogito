//! MCP (Model Context Protocol) server for Cogito.
//!
//! Runs over stdio so any MCP-capable agent (Claude Code, Cursor, Codex,
//! opencode, Gemini CLI, …) can use Cogito as a tool provider.
//!
//! The tools give an agent the project's layout so it does not have to read
//! the code to find its way: `project_map`, `related_files`, and `search_code`.
//!
//! Each tool takes an explicit `path` argument. The server opens that
//! project's `.cogito/state.db` per call, so one process can serve several
//! projects without mixing their data.

use anyhow::Result;
use cogito_harness_core::{Fingerprint, Project};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};
use std::path::PathBuf;

/// Server name advertised to MCP clients.
pub const SERVER_NAME: &str = "cogito";

/// What the client tells the model about this server.
const INSTRUCTIONS: &str = "Cogito indexes the project's code so you can answer without reading \
it. Cheapest path first: project_map to orient (once per session); search_code to find a \
symbol, file or module by name (it gives file:line); file_outline to see a file's declarations \
with line ranges; then read only the lines you need. related_files shows what a file imports \
and what imports it. Avoid listing directories, grepping or reading whole files to find your \
way. The index refreshes when HEAD changes; pass refresh=true to project_map after large \
uncommitted changes. Every tool takes `path`, the absolute project root.";

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
    /// Words to look for in file paths and module names, e.g. `auth session`
    pub query: String,
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
fn open_project(path: &str) -> Result<(cogito_harness_core::Cogito, Project), String> {
    let root = PathBuf::from(path);
    let cogito = cogito_harness_core::Cogito::for_project(&root)
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
) -> Result<(cogito_harness_core::Cogito, Project, Fingerprint), String> {
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
        description = "A file's declarations (code) or sections (Markdown) with line ranges. Read only the range you need instead of the whole file."
    )]
    async fn file_outline(
        &self,
        Parameters(req): Parameters<RelatedFilesRequest>,
    ) -> Result<String, String> {
        let (_, project, fingerprint) = mapped_project(&req.path, false).await?;
        let file = cogito_harness_core::map::find_file(&project.path, &fingerprint, &req.file)?;
        cogito_harness_core::map::render_outline(&project.path, &fingerprint, &file)
    }

    #[tool(
        description = "Find code or docs by words: symbols (functions, types, classes) and doc sections with file:line, and file paths (most changed first)."
    )]
    async fn search_code(
        &self,
        Parameters(req): Parameters<SearchCodeRequest>,
    ) -> Result<String, String> {
        let (_, project, fingerprint) = mapped_project(&req.path, false).await?;
        let found = cogito_harness_core::map::search_paths(&project.path, &fingerprint, &req.query);
        Ok(cogito_harness_core::map::render_search(&found, &req.query))
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
