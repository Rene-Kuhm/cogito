//! Cogito REST API Server
//!
//! Provides HTTP API for context optimization operations.

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use cogito_core::{Cogito, CompressionOptions};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Application state
#[derive(Clone)]
struct AppState {
    cogito: Arc<Cogito>,
}

#[derive(Serialize)]
struct ApiResponse<T> {
    success: bool,
    data: Option<T>,
    error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    fn error(message: String) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(message),
        }
    }
}

/// Parse a project id from the request; a malformed one is the client's error.
// The Err is the ready-made 400 response: large, but it goes straight back to axum.
#[allow(clippy::result_large_err)]
fn parse_project_id(raw: &str) -> Result<uuid::Uuid, axum::response::Response> {
    raw.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<()>::error(format!(
                "invalid project id: {raw}"
            ))),
        )
            .into_response()
    })
}

// --- Project Endpoints ---

/// Create a new project
async fn create_project(
    State(state): State<AppState>,
    Json(payload): Json<CreateProjectRequest>,
) -> impl IntoResponse {
    let result = state.cogito.init_project(payload.path.into());
    match result {
        Ok(project) => (StatusCode::CREATED, Json(ApiResponse::success(project))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<()>::error(e.to_string())),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct CreateProjectRequest {
    path: String,
}

/// Get project by ID
async fn get_project(State(_state): State<AppState>, Path(_id): Path<String>) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(ApiResponse::<()>::error("Not implemented".into())),
    )
        .into_response()
}

/// Delete project
async fn delete_project(
    State(_state): State<AppState>,
    Path(_id): Path<String>,
) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(ApiResponse::<()>::error("Not implemented".into())),
    )
        .into_response()
}

// --- Context Endpoints ---

/// Compress context
async fn compress_context(
    State(state): State<AppState>,
    Json(payload): Json<CompressRequest>,
) -> impl IntoResponse {
    let options = CompressionOptions {
        max_tokens: payload.max_tokens.unwrap_or(8000),
        preserve_imports: payload.preserve_imports.unwrap_or(true),
        preserve_types: payload.preserve_types.unwrap_or(true),
        hot_zones_priority: payload.hot_zones_priority.unwrap_or(true),
    };

    let result = state
        .cogito
        .compress_context(&payload.content, options)
        .await;
    match result {
        Ok(data) => (StatusCode::OK, Json(ApiResponse::success(data))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<()>::error(e.to_string())),
        )
            .into_response(),
    }
}

// project_id, query and include_* are part of the documented request but the
// compressor does not use them yet.
#[derive(Deserialize)]
#[allow(dead_code)]
struct CompressRequest {
    project_id: Option<String>,
    content: String,
    max_tokens: Option<usize>,
    preserve_imports: Option<bool>,
    preserve_types: Option<bool>,
    hot_zones_priority: Option<bool>,
    query: Option<String>,
    include_decisions: Option<bool>,
    include_tags: Option<bool>,
}

/// Estimate compression
async fn estimate_compression(
    State(_state): State<AppState>,
    Json(payload): Json<EstimateRequest>,
) -> impl IntoResponse {
    let response = EstimateResponse {
        estimated_tokens: payload.content.len() / 3,
        estimated_savings: 30.0,
    };
    (StatusCode::OK, Json(ApiResponse::success(response))).into_response()
}

#[derive(Deserialize)]
struct EstimateRequest {
    content: String,
}

#[derive(Serialize)]
struct EstimateResponse {
    estimated_tokens: usize,
    estimated_savings: f32,
}

// --- Decision Endpoints ---

/// Create decision
async fn create_decision(
    State(state): State<AppState>,
    Json(payload): Json<CreateDecisionRequest>,
) -> impl IntoResponse {
    use cogito_core::decision;

    let project_id = match parse_project_id(&payload.project_id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let mut builder = decision::builder(project_id)
        .title(&payload.title)
        .why(&payload.why)
        .author(&payload.author.unwrap_or_else(whoami::username));
    for alternative in payload.alternatives.unwrap_or_default() {
        builder = builder.alternative(&alternative);
    }
    for path in payload.paths.unwrap_or_default() {
        builder = builder.path(path.into());
    }
    for consequence in payload.consequences.unwrap_or_default() {
        builder = builder.consequence(&consequence);
    }
    for tag in payload.tags.unwrap_or_default() {
        builder = builder.tag(&tag);
    }
    let decision = builder.build();

    let result = state.cogito.add_decision(decision).await;
    match result {
        Ok(data) => (StatusCode::CREATED, Json(ApiResponse::success(data))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<()>::error(e.to_string())),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct CreateDecisionRequest {
    project_id: String,
    title: String,
    why: String,
    alternatives: Option<Vec<String>>,
    paths: Option<Vec<String>>,
    consequences: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    author: Option<String>,
}

/// List decisions
async fn list_decisions(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> impl IntoResponse {
    let project_id = match parse_project_id(&params.project) {
        Ok(id) => id,
        Err(response) => return response,
    };
    // An empty query matches every decision, the same way list_tags lists tags.
    let result = state.cogito.search_decisions(project_id, "").await;
    match result {
        Ok(data) => (StatusCode::OK, Json(ApiResponse::success(data))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<()>::error(e.to_string())),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ListParams {
    project: String,
}

/// Search decisions
async fn search_decisions(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let project_id = match parse_project_id(&params.project) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let result = state.cogito.search_decisions(project_id, &params.q).await;
    match result {
        Ok(data) => (StatusCode::OK, Json(ApiResponse::success(data))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<()>::error(e.to_string())),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct SearchParams {
    project: String,
    q: String,
}

// --- Tag Endpoints ---

/// Create tag
async fn create_tag(
    State(state): State<AppState>,
    Json(payload): Json<CreateTagRequest>,
) -> impl IntoResponse {
    use chrono::Utc;
    use cogito_core::types::SemanticTag;
    use uuid::Uuid;

    let project_id = match parse_project_id(&payload.project_id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let tag = SemanticTag {
        id: Uuid::new_v4(),
        project_id,
        tag: payload.tag,
        domain: payload.domain.unwrap_or_default(),
        paths: payload.paths.into_iter().map(Into::into).collect(),
        confidence: payload.confidence.unwrap_or(0.8),
        created_at: Utc::now(),
    };

    let result = state.cogito.add_tag(tag).await;
    match result {
        Ok(data) => (StatusCode::CREATED, Json(ApiResponse::success(data))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<()>::error(e.to_string())),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct CreateTagRequest {
    project_id: String,
    tag: String,
    domain: Option<String>,
    paths: Vec<String>,
    confidence: Option<f32>,
}

/// List tags
async fn list_tags(
    State(state): State<AppState>,
    Query(params): Query<TagListParams>,
) -> impl IntoResponse {
    let project_id = match parse_project_id(&params.project) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let result = state.cogito.search_tags(project_id, "").await;
    match result {
        Ok(data) => (StatusCode::OK, Json(ApiResponse::success(data))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<()>::error(e.to_string())),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct TagListParams {
    project: String,
}

/// Search tags
async fn search_tags(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    let project_id = match parse_project_id(&params.project) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let result = state.cogito.search_tags(project_id, &params.q).await;
    match result {
        Ok(data) => (StatusCode::OK, Json(ApiResponse::success(data))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<()>::error(e.to_string())),
        )
            .into_response(),
    }
}

// --- Health Endpoint ---

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::new("info"))
        .init();

    // Create application state
    let state = AppState {
        cogito: Arc::new(Cogito::new()),
    };

    // Setup CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Build router
    let app = Router::new()
        .route("/health", get(health))
        // Projects
        .route("/api/v1/projects", post(create_project))
        .route("/api/v1/projects/:id", get(get_project))
        .route(
            "/api/v1/projects/:id",
            axum::routing::delete(delete_project),
        )
        .route("/api/v1/projects/:id/scan", post(create_project))
        // Context
        .route("/api/v1/context/compress", post(compress_context))
        .route("/api/v1/context/estimate", post(estimate_compression))
        // Decisions
        .route("/api/v1/decisions", post(create_decision))
        .route("/api/v1/decisions", get(list_decisions))
        .route("/api/v1/decisions/search", get(search_decisions))
        // Tags
        .route("/api/v1/tags", post(create_tag))
        .route("/api/v1/tags", get(list_tags))
        .route("/api/v1/tags/search", get(search_tags))
        .with_state(state)
        .layer(cors);

    // Start server
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    tracing::info!("Starting Cogito API server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
