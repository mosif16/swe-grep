use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde::Serialize;
use tokio::fs;
use tokio::time::Instant;

use crate::cli::{BenchArgs, SearchArgs};
use crate::search;

pub async fn run(args: BenchArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let scenario_path = args
        .scenario
        .clone()
        .unwrap_or_else(|| PathBuf::from("benchmarks/default.json"));
    let scenario_path = if scenario_path.is_absolute() {
        scenario_path
    } else {
        cwd.join(scenario_path)
    };

    let raw = fs::read_to_string(&scenario_path).await.with_context(|| {
        format!(
            "failed to read benchmark scenarios from {}",
            scenario_path.display()
        )
    })?;
    let scenarios: Vec<Scenario> = serde_json::from_str(&raw).with_context(|| {
        format!(
            "failed to parse benchmark scenarios in {}",
            scenario_path.display()
        )
    })?;

    let iterations = usize::max(1, args.iterations);
    let mut reports = Vec::new();
    let mut total_elapsed = Duration::ZERO;
    let mut total_iterations = 0usize;
    let mut total_hits = 0usize;

    for scenario in scenarios {
        let repo_root = resolve_path(&cwd, &scenario.path).await?;
        let mut latencies = Vec::<f64>::new();
        let mut hits = 0usize;
        let mut latest_top_hits = Vec::new();

        for _ in 0..iterations {
            let search_args = build_search_args(&repo_root, &scenario, &args);
            let start = Instant::now();
            let summary = search::execute(search_args).await?;
            let elapsed = start.elapsed();

            latencies.push(elapsed.as_secs_f64() * 1000.0);
            total_elapsed += elapsed;
            total_iterations += 1;

            let matched = matches_expectation(&summary, &scenario);
            if matched {
                hits += 1;
                total_hits += 1;
            }

            latest_top_hits = summary.top_hits.clone();
        }

        let mean_latency_ms = if latencies.is_empty() {
            0.0
        } else {
            latencies.iter().copied().sum::<f64>() / latencies.len() as f64
        };
        let success_rate = if latencies.is_empty() {
            0.0
        } else {
            hits as f64 / latencies.len() as f64
        };
        let throughput_qps = if mean_latency_ms > 0.0 {
            1000.0 / mean_latency_ms
        } else {
            0.0
        };

        reports.push(ScenarioReport {
            name: scenario.name.clone(),
            symbol: scenario.symbol.clone(),
            iterations: latencies.len(),
            mean_latency_ms,
            throughput_qps,
            success_rate,
            hits,
            expected: scenario.expected.clone(),
            latest_top_hits,
        });
    }

    let overall_mean_latency_ms = if total_iterations == 0 {
        0.0
    } else {
        (total_elapsed.as_secs_f64() * 1000.0) / total_iterations as f64
    };
    let overall_qps = if total_elapsed.is_zero() {
        0.0
    } else {
        total_iterations as f64 / total_elapsed.as_secs_f64()
    };
    let overall_success_rate = if total_iterations == 0 {
        0.0
    } else {
        total_hits as f64 / total_iterations as f64
    };

    let summary = BenchmarkSummary {
        scenarios: reports,
        totals: Totals {
            total_iterations,
            total_hits,
            mean_latency_ms: overall_mean_latency_ms,
            throughput_qps: overall_qps,
            success_rate: overall_success_rate,
        },
    };

    let rendered = serde_json::to_string_pretty(&summary)?;
    println!("{}", rendered);

    if let Some(output_path) = args.output {
        let mut path = if output_path.is_absolute() {
            output_path
        } else {
            cwd.join(output_path)
        };
        if path.is_dir() {
            path = path.join("benchmark-summary.jsonl");
        }
        fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new(".")))
            .await
            .with_context(|| {
                format!(
                    "failed to create benchmark output directory {}",
                    path.display()
                )
            })?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .with_context(|| format!("failed to open benchmark output file {}", path.display()))?;
        let mut line = serde_json::to_vec(&summary)?;
        line.push(b'\n');
        use tokio::io::AsyncWriteExt;
        file.write_all(&line).await?;
    }

    Ok(())
}

fn build_search_args(repo_root: &Path, scenario: &Scenario, bench: &BenchArgs) -> SearchArgs {
    let enable_index = scenario.enable_index.unwrap_or(bench.enable_index);
    let enable_rga = scenario.enable_rga.unwrap_or(bench.enable_rga);
    let index_dir = scenario
        .index_dir
        .clone()
        .or_else(|| bench.cache_dir.clone().map(|dir| dir.join("index")))
        .unwrap_or_else(|| repo_root.join(".swe-grep-index"));

    let cache_dir = scenario
        .cache_dir
        .clone()
        .or_else(|| bench.cache_dir.clone());

    let log_dir = scenario.log_dir.clone().or_else(|| bench.log_dir.clone());

    SearchArgs {
        symbol: scenario.symbol.clone(),
        path: Some(repo_root.to_path_buf()),
        language: scenario.language.clone(),
        timeout_secs: scenario.timeout_secs.unwrap_or(3),
        max_matches: scenario.max_matches.unwrap_or(20),
        concurrency: scenario.concurrency.unwrap_or(8),
        enable_index,
        index_dir: Some(index_dir),
        enable_rga,
        cache_dir,
        log_dir,
        use_fd: true,
        use_ast_grep: true,
    }
}

async fn resolve_path(base: &Path, path: &Path) -> Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let canonical = fs::canonicalize(&joined)
        .await
        .with_context(|| format!("failed to canonicalize path {}", joined.display()))?;
    Ok(canonical)
}

fn matches_expectation(summary: &search::SearchSummary, scenario: &Scenario) -> bool {
    if let Some(expected) = &scenario.expected {
        let top_n = expected.top_n.unwrap_or(1);
        summary.top_hits.iter().take(top_n).any(|hit| {
            path_matches(&hit.path, &expected.path)
                && expected.line.map_or(true, |line| line == hit.line)
        })
    } else {
        !summary.top_hits.is_empty()
    }
}

fn path_matches(hit_path: &str, expected: &str) -> bool {
    let hit = Path::new(hit_path);
    let expected_path = Path::new(expected);
    hit == expected_path || hit.ends_with(expected_path)
}

#[derive(Clone, Deserialize, Serialize)]
struct Scenario {
    name: String,
    path: PathBuf,
    symbol: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    expected: Option<Expectation>,
    #[serde(default)]
    enable_index: Option<bool>,
    #[serde(default)]
    enable_rga: Option<bool>,
    #[serde(default)]
    cache_dir: Option<PathBuf>,
    #[serde(default)]
    log_dir: Option<PathBuf>,
    #[serde(default)]
    index_dir: Option<PathBuf>,
    #[serde(default)]
    concurrency: Option<usize>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    max_matches: Option<usize>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Expectation {
    path: String,
    #[serde(default)]
    line: Option<usize>,
    #[serde(default)]
    top_n: Option<usize>,
}

#[derive(Serialize)]
struct BenchmarkSummary {
    scenarios: Vec<ScenarioReport>,
    totals: Totals,
}

#[derive(Serialize)]
struct ScenarioReport {
    name: String,
    symbol: String,
    iterations: usize,
    mean_latency_ms: f64,
    throughput_qps: f64,
    success_rate: f64,
    hits: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<Expectation>,
    latest_top_hits: Vec<crate::search::TopHit>,
}

#[derive(Serialize)]
struct Totals {
    total_iterations: usize,
    total_hits: usize,
    mean_latency_ms: f64,
    throughput_qps: f64,
    success_rate: f64,
}
