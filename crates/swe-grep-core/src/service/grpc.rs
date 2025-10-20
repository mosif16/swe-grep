use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tonic::async_trait;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use crate::search::{SearchSummary, StageStats};

use super::proto::{
    self,
    swe_grep_service_server::{SweGrepService, SweGrepServiceServer},
};
use super::server::{SearchExecutor, SearchInput};

/// Start the gRPC server and block until shutdown.
pub async fn serve(addr: SocketAddr, executor: Arc<SearchExecutor>) -> Result<()> {
    let service = SweGrepGrpc { executor };

    Server::builder()
        .add_service(SweGrepServiceServer::new(service))
        .serve_with_shutdown(addr, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .with_context(|| format!("failed to start gRPC server on {addr}"))
}

#[derive(Clone)]
struct SweGrepGrpc {
    executor: Arc<SearchExecutor>,
}

#[async_trait]
impl SweGrepService for SweGrepGrpc {
    async fn search(
        &self,
        request: Request<proto::SearchRequest>,
    ) -> Result<Response<proto::SearchResponse>, Status> {
        let inner = request.into_inner();
        let input = map_request(inner);

        let summary = self.executor.execute(input).await.map_err(|err| {
            let msg = err.to_string();
            if msg.contains("symbol is required") {
                Status::invalid_argument(msg)
            } else {
                Status::internal(msg)
            }
        })?;

        let response = proto::SearchResponse {
            summary: Some(summary.into()),
        };

        Ok(Response::new(response))
    }

    async fn health(
        &self,
        _request: Request<proto::HealthCheckRequest>,
    ) -> Result<Response<proto::HealthCheckResponse>, Status> {
        let response = proto::HealthCheckResponse {
            status: "ok".to_string(),
        };
        Ok(Response::new(response))
    }
}

fn map_request(proto: proto::SearchRequest) -> SearchInput {
    SearchInput {
        symbol: proto.symbol,
        language: option_from_string(proto.language),
        root: path_from_string(proto.root),
        timeout_secs: zeroable(proto.timeout_secs),
        max_matches: zeroable_usize(proto.max_matches),
        concurrency: zeroable_usize(proto.concurrency),
        enable_index: Some(proto.enable_index).filter(|value| *value != false),
        enable_rga: Some(proto.enable_rga).filter(|value| *value != false),
        index_dir: path_from_string(proto.index_dir),
        cache_dir: path_from_string(proto.cache_dir),
        log_dir: path_from_string(proto.log_dir),
        tool_flags: proto.tool_flags,
    }
}

fn option_from_string(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn path_from_string(value: String) -> Option<PathBuf> {
    option_from_string(value).map(PathBuf::from)
}

fn zeroable(value: u32) -> Option<u64> {
    if value == 0 { None } else { Some(value as u64) }
}

fn zeroable_usize(value: u32) -> Option<usize> {
    if value == 0 {
        None
    } else {
        Some(value as usize)
    }
}

impl From<SearchSummary> for proto::SearchSummary {
    fn from(summary: SearchSummary) -> Self {
        let stage_stats = Some(convert_stage_stats(summary.stage_stats));

        proto::SearchSummary {
            cycle: summary.cycle,
            symbol: summary.symbol,
            queries: summary.queries,
            top_hits: summary
                .top_hits
                .into_iter()
                .map(|hit| proto::TopHit {
                    path: hit.path,
                    line: hit.line as u32,
                    score: hit.score,
                    origin: hit.origin,
                    snippet: hit.snippet.unwrap_or_default(),
                })
                .collect(),
            deduped: summary.deduped as u32,
            next_actions: summary.next_actions,
            fd_candidates: summary
                .fd_candidates
                .into_iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect(),
            ast_hits: summary
                .ast_hits
                .into_iter()
                .map(|(path, line)| proto::AstHit {
                    path: path.to_string_lossy().to_string(),
                    line: line as u32,
                })
                .collect(),
            stage_stats,
            reward: summary.reward,
        }
    }
}

fn convert_stage_stats(stats: StageStats) -> proto::StageStats {
    proto::StageStats {
        discover_candidates: stats.discover_candidates as u32,
        discover_ms: stats.discover_ms,
        probe_hits: stats.probe_hits as u32,
        probe_ms: stats.probe_ms,
        escalate_hits: stats.escalate_hits as u32,
        escalate_ms: stats.escalate_ms,
        index_candidates: stats.index_candidates as u32,
        index_probe_hits: stats.index_probe_hits as u32,
        index_ms: stats.index_ms,
        rga_hits: stats.rga_hits as u32,
        rga_ms: stats.rga_ms,
        ast_matches: stats.ast_matches as u32,
        disambiguate_ms: stats.disambiguate_ms,
        verify_ms: stats.verify_ms,
        cycle_latency_ms: stats.cycle_latency_ms,
        precision: stats.precision,
        density: stats.density,
        clustering: stats.clustering,
        reward: stats.reward,
    }
}
