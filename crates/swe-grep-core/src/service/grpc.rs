use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tonic::async_trait;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use crate::search::{ConceptExpansionSummary, GitInfo, SearchSummary, StageStats, StartupStats};

use super::proto::{
    self,
    swe_grep_service_server::{SweGrepService, SweGrepServiceServer},
};
use super::server::{SearchExecutor, SearchInput, ToolFlagOverrides};

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
        concept: option_from_string(proto.concept),
        concept_expand: Some(proto.concept_expand),
        language: option_from_string(proto.language),
        kind: option_from_string(proto.kind),
        root: path_from_string(proto.root),
        timeout_secs: zeroable(proto.timeout_secs),
        max_matches: zeroable_usize(proto.max_matches),
        concurrency: zeroable_usize(proto.concurrency),
        enable_index: Some(proto.enable_index),
        enable_rga: Some(proto.enable_rga),
        regex: Some(proto.regex),
        wildcard: Some(proto.wildcard),
        string: Some(proto.string),
        include_strings: Some(proto.include_strings),
        code_only: Some(proto.code_only),
        ignore_case: Some(proto.ignore_case),
        definitions_only: Some(proto.definitions_only),
        usages_only: Some(proto.usages_only),
        path_glob: if proto.path_glob.is_empty() {
            None
        } else {
            Some(proto.path_glob)
        },
        exclude_patterns: if proto.exclude.is_empty() {
            None
        } else {
            Some(proto.exclude)
        },
        index_dir: path_from_string(proto.index_dir),
        cache_dir: path_from_string(proto.cache_dir),
        log_dir: path_from_string(proto.log_dir),
        context_before: zeroable_usize(proto.context_before),
        context_after: zeroable_usize(proto.context_after),
        body: Some(proto.body),
        git_modified: Some(proto.git_modified),
        git_since: option_from_string(proto.git_since),
        git_info: Some(proto.git_info),
        exclude_tests: Some(proto.exclude_tests),
        tool_flags: ToolFlagOverrides::from_map(proto.tool_flags),
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
        let startup_stats = summary.startup_stats.map(convert_startup_stats);
        let max_matches = summary.max_matches.min(u32::MAX as usize) as u32;

        proto::SearchSummary {
            cycle: summary.cycle,
            symbol: summary.symbol,
            queries: summary.queries,
            top_hits: summary
                .top_hits
                .into_iter()
                .map(|hit| {
                    let snippet_length =
                        hit.snippet_length.unwrap_or(0).min(u32::MAX as usize) as u32;
                    let context_start =
                        hit.context_start.unwrap_or(0).min(u32::MAX as usize) as u32;
                    let context_end = hit.context_end.unwrap_or(0).min(u32::MAX as usize) as u32;

                    proto::TopHit {
                        path: hit.path,
                        line: hit.line as u32,
                        score: hit.score,
                        origin: hit.origin,
                        origin_label: hit.origin_label,
                        snippet: hit.snippet.unwrap_or_default(),
                        raw_snippet: hit.raw_snippet.unwrap_or_default(),
                        snippet_length,
                        raw_snippet_truncated: hit.raw_snippet_truncated,
                        expanded_snippet: hit.expanded_snippet.unwrap_or_default(),
                        context_start,
                        context_end,
                        body: hit.body.unwrap_or_default(),
                        body_retrieved: hit.body_retrieved,
                        auto_expanded_context: hit.auto_expanded_context,
                        git_info: hit.git_info.map(proto::GitInfo::from),
                        hints: hit
                            .hints
                            .into_iter()
                            .map(|hint| proto::ContextHint {
                                kind: hint.kind,
                                label: hint.label,
                                line: hint.line as u32,
                            })
                            .collect(),
                    }
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
            startup_stats,
            warnings: summary.warnings,
            root: summary.root.to_string_lossy().to_string(),
            concept: summary.concept.unwrap_or_default(),
            concept_expansion: summary
                .concept_expansion
                .map(proto::ConceptExpansionSummary::from),
            max_matches,
            related_symbols: summary.related_symbols,
            suggested_symbols: summary.suggested_symbols,
            related_paths: summary.related_paths,
            notes: summary.notes,
            open_commands: summary.open_commands,
            truncated: summary.truncated,
            ast_fallback: summary.ast_fallback,
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

fn convert_startup_stats(stats: StartupStats) -> proto::StartupStats {
    proto::StartupStats {
        init_ms: stats.init_ms,
        fd_ms: stats.fd_ms,
        rg_ms: stats.rg_ms,
        ast_ms: stats.ast_ms,
        rga_ms: stats.rga_ms,
        cache_ms: stats.cache_ms,
        state_ms: stats.state_ms,
        index_ms: stats.index_ms,
    }
}

impl From<GitInfo> for proto::GitInfo {
    fn from(info: GitInfo) -> Self {
        proto::GitInfo {
            last_commit: info.last_commit,
            last_modified: info.last_modified,
        }
    }
}

impl From<ConceptExpansionSummary> for proto::ConceptExpansionSummary {
    fn from(summary: ConceptExpansionSummary) -> Self {
        proto::ConceptExpansionSummary {
            primary: summary.primary,
            synonyms: summary.synonyms,
            phrases: summary.phrases,
        }
    }
}
