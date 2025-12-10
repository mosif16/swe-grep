use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{Response, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::search::SearchSummary;

use super::server::{SearchExecutor, SearchInput, ToolFlagOverrides};

type SharedExecutor = Arc<SearchExecutor>;

#[derive(Debug, Deserialize)]
pub struct HttpSearchRequest {
    pub symbol: String,
    #[serde(default)]
    pub concept: Option<String>,
    #[serde(default)]
    pub concept_expand: Option<bool>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_matches: Option<usize>,
    #[serde(default)]
    pub concurrency: Option<usize>,
    #[serde(default)]
    pub enable_index: Option<bool>,
    #[serde(default)]
    pub enable_rga: Option<bool>,
    #[serde(default)]
    pub index_dir: Option<String>,
    #[serde(default)]
    pub cache_dir: Option<String>,
    #[serde(default)]
    pub log_dir: Option<String>,
    #[serde(default)]
    pub tool_flags: HashMap<String, bool>,
    #[serde(default)]
    pub use_fd: Option<bool>,
    #[serde(default)]
    pub use_ast_grep: Option<bool>,
    #[serde(default)]
    pub use_index: Option<bool>,
    #[serde(default)]
    pub use_rga: Option<bool>,
    #[serde(default)]
    pub context_before: Option<usize>,
    #[serde(default)]
    pub context_after: Option<usize>,
    #[serde(default)]
    pub body: Option<bool>,
    #[serde(default)]
    pub git_info: Option<bool>,
    #[serde(default)]
    pub regex: Option<bool>,
    #[serde(default)]
    pub wildcard: Option<bool>,
    #[serde(default)]
    pub string: Option<bool>,
    #[serde(default)]
    pub include_strings: Option<bool>,
    #[serde(default)]
    pub code_only: Option<bool>,
    #[serde(default)]
    pub ignore_case: Option<bool>,
    #[serde(default)]
    pub definitions_only: Option<bool>,
    #[serde(default)]
    pub usages_only: Option<bool>,
    #[serde(default)]
    pub path_glob: Option<Vec<String>>,
    #[serde(default)]
    pub exclude: Option<Vec<String>>,
    #[serde(default)]
    pub exclude_tests: Option<bool>,
    #[serde(default)]
    pub git_modified: Option<bool>,
    #[serde(default)]
    pub git_since: Option<String>,
}

impl From<HttpSearchRequest> for SearchInput {
    fn from(req: HttpSearchRequest) -> Self {
        let mut tool_flags = ToolFlagOverrides::from_map(req.tool_flags);
        tool_flags.set_fd(req.use_fd);
        tool_flags.set_ast_grep(req.use_ast_grep);
        tool_flags.set_index(req.use_index);
        tool_flags.set_rga(req.use_rga);
        tool_flags.set_body(req.body);

        SearchInput {
            symbol: req.symbol,
            concept: req.concept,
            concept_expand: req.concept_expand,
            language: req.language,
            kind: req.kind,
            root: req.root.map(PathBuf::from),
            timeout_secs: req.timeout_secs,
            max_matches: req.max_matches,
            concurrency: req.concurrency,
            enable_index: req.enable_index,
            enable_rga: req.enable_rga,
            regex: req.regex,
            wildcard: req.wildcard,
            string: req.string,
            include_strings: req.include_strings,
            code_only: req.code_only,
            ignore_case: req.ignore_case,
            definitions_only: req.definitions_only,
            usages_only: req.usages_only,
            path_glob: req.path_glob,
            exclude_patterns: req.exclude,
            exclude_tests: req.exclude_tests,
            index_dir: req.index_dir.map(PathBuf::from),
            cache_dir: req.cache_dir.map(PathBuf::from),
            log_dir: req.log_dir.map(PathBuf::from),
            tool_flags,
            context_before: req.context_before,
            context_after: req.context_after,
            body: req.body,
            git_modified: req.git_modified,
            git_since: req.git_since,
            git_info: req.git_info,
        }
    }
}

#[derive(Serialize)]
pub struct HttpSearchResponse {
    pub summary: SearchSummary,
}

#[derive(Serialize)]
struct ErrorResponse {
    message: String,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

/// Start the HTTP server and run until shutdown.
pub async fn serve(addr: SocketAddr, executor: SharedExecutor) -> Result<()> {
    let app = Router::new()
        .route("/healthz", get(health))
        .route("/search", post(search))
        .route("/metrics", get(metrics))
        .with_state(executor);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind HTTP address {addr}"))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .with_context(|| format!("failed to run HTTP server on {addr}"))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn search(
    State(executor): State<SharedExecutor>,
    Json(request): Json<HttpSearchRequest>,
) -> Result<Json<HttpSearchResponse>, (StatusCode, Json<ErrorResponse>)> {
    if request.symbol.trim().is_empty()
        && request
            .concept
            .as_deref()
            .map(|c| c.trim().is_empty())
            .unwrap_or(true)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                message: "symbol is required".to_string(),
            }),
        ));
    }

    let input: SearchInput = request.into();

    match executor.execute(input).await {
        Ok(summary) => Ok(Json(HttpSearchResponse { summary })),
        Err(err) => {
            let msg = err.to_string();
            let status = if msg.contains("symbol is required") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            Err((status, Json(ErrorResponse { message: msg })))
        }
    }
}

async fn metrics() -> Result<Response<Body>, StatusCode> {
    match crate::telemetry::export_prometheus() {
        Ok(body) => Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")
            .body(Body::from(body))
            .map_err(|err| {
                tracing::error!(error = %err, "failed to build metrics response");
                StatusCode::INTERNAL_SERVER_ERROR
            }),
        Err(err) => {
            tracing::error!(error = %err, "failed to export metrics");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
