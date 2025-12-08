use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use grep_matcher::LineTerminator;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
use ignore::WalkBuilder;
use lru::LruCache;
use rayon::ThreadPoolBuilder;
use rayon::prelude::*;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::task;
use tokio::time::timeout;

use super::common::{ChildGuard, RgMessage};

const MATCHER_CACHE_CAPACITY: usize = 64;

#[derive(Clone, Debug)]
pub struct RipgrepTool {
    timeout: Duration,
    max_matches: usize,
    context_before: usize,
    context_after: usize,
    max_columns: usize,
    threads: usize,
    matcher_cache: Arc<Mutex<LruCache<String, RegexMatcher>>>,
}

impl RipgrepTool {
    pub fn new(
        timeout: Duration,
        max_matches: usize,
        context_before: usize,
        context_after: usize,
        max_columns: usize,
        threads: usize,
    ) -> Self {
        let cache_size =
            NonZeroUsize::new(MATCHER_CACHE_CAPACITY).unwrap_or(NonZeroUsize::new(1).unwrap());
        Self {
            timeout,
            max_matches,
            context_before,
            context_after,
            max_columns,
            threads: usize::max(1, threads),
            matcher_cache: Arc::new(Mutex::new(LruCache::new(cache_size))),
        }
    }

    pub async fn search_union(
        &self,
        root: &Path,
        queries: &[String],
        paths: &[PathBuf],
    ) -> Result<Vec<RipgrepMatch>> {
        if queries.is_empty() {
            return Ok(Vec::new());
        }

        match self.search_embedded(root, queries, paths).await {
            Ok(results) => Ok(results),
            Err(embed_err) => {
                tracing::warn!(
                    error = %embed_err,
                    "embedded ripgrep failed; falling back to external rg"
                );
                match self.search_external(root, queries, paths).await {
                    Ok(results) => Ok(results),
                    Err(fallback_err) => Err(embed_err
                        .context(format!("fallback to external rg failed: {fallback_err}"))),
                }
            }
        }
    }

    async fn search_embedded(
        &self,
        root: &Path,
        queries: &[String],
        paths: &[PathBuf],
    ) -> Result<Vec<RipgrepMatch>> {
        let deadline = Instant::now() + self.timeout;
        let matcher = self.compile_matcher(queries)?;
        let search_paths = self.collect_paths(root, paths)?;
        if search_paths.is_empty() {
            return Ok(Vec::new());
        }

        if Instant::now() >= deadline {
            return Err(anyhow!("embedded ripgrep search timed out"));
        }

        let builder = self.searcher_builder();
        let max_matches = self.max_matches;
        let max_columns = self.max_columns;
        let threads = self.threads;
        let root = root.to_path_buf();
        let remaining = deadline.saturating_duration_since(Instant::now());

        let handle = task::spawn_blocking(move || {
            run_embedded_search(
                matcher,
                builder,
                root,
                search_paths,
                threads,
                max_matches,
                max_columns,
                deadline,
            )
        });

        match timeout(remaining, handle).await {
            Ok(result) => result.context("embedded ripgrep task failed to complete")?,
            Err(_) => Err(anyhow!("embedded ripgrep search timed out")),
        }
    }

    fn searcher_builder(&self) -> SearcherBuilder {
        let mut builder = SearcherBuilder::new();
        builder
            .line_number(true)
            .before_context(self.context_before)
            .after_context(self.context_after)
            .max_matches(Some(self.max_matches as u64))
            .line_terminator(LineTerminator::byte(b'\n'))
            .binary_detection(BinaryDetection::quit(b'\x00'));
        builder
    }

    fn compile_matcher(&self, queries: &[String]) -> Result<RegexMatcher> {
        let mut normalized: Vec<String> = queries.to_vec();
        normalized.sort();
        let key = normalized.join("\u{1f}");

        if let Some(cached) = self.matcher_cache.lock().unwrap().get(&key) {
            return Ok(cached.clone());
        }

        let mut builder = RegexMatcherBuilder::new();
        builder.case_smart(true).line_terminator(Some(b'\n'));
        let matcher = builder
            .build_many(&normalized)
            .context("failed to compile ripgrep patterns")?;
        self.matcher_cache.lock().unwrap().put(key, matcher.clone());
        Ok(matcher)
    }

    fn collect_paths(&self, root: &Path, paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
        let mut roots: Vec<PathBuf> = Vec::new();
        if paths.is_empty() {
            roots.push(root.to_path_buf());
        } else {
            for path in paths.iter() {
                let absolute = if path.is_absolute() {
                    path.clone()
                } else {
                    root.join(path)
                };
                roots.push(absolute);
            }
        }

        let mut builder =
            WalkBuilder::new(roots.get(0).cloned().unwrap_or_else(|| root.to_path_buf()));
        for extra in roots.iter().skip(1) {
            builder.add(extra);
        }
        builder
            .hidden(true)
            .ignore(true)
            .parents(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .follow_links(false);

        let mut files = Vec::new();
        let mut seen = HashSet::new();
        for entry in builder.build() {
            match entry {
                Ok(dir_entry) => {
                    if dir_entry
                        .file_type()
                        .map(|ft| ft.is_file())
                        .unwrap_or(false)
                    {
                        let path = dir_entry.into_path();
                        if seen.insert(path.clone()) {
                            files.push(path);
                        }
                    }
                }
                Err(err) => {
                    tracing::debug!(error = %err, "skipping path due to walker error");
                }
            }
        }

        Ok(files)
    }

    async fn search_external(
        &self,
        root: &Path,
        queries: &[String],
        paths: &[PathBuf],
    ) -> Result<Vec<RipgrepMatch>> {
        let mut cmd = Command::new("rg");
        cmd.arg("--json")
            .arg("--line-number")
            .arg("--column")
            .arg("--threads")
            .arg(self.threads.to_string())
            .arg("--max-columns")
            .arg(self.max_columns.to_string())
            .arg("--smart-case")
            .arg("--max-count")
            .arg(self.max_matches.to_string());

        if self.context_before > 0 {
            cmd.arg("--before-context")
                .arg(self.context_before.to_string());
        }
        if self.context_after > 0 {
            cmd.arg("--after-context")
                .arg(self.context_after.to_string());
        }

        for query in queries {
            cmd.arg("-e").arg(query);
        }

        if paths.is_empty() {
            cmd.arg(".");
        } else {
            for path in paths.iter() {
                let absolute = if path.is_absolute() {
                    path.clone()
                } else {
                    root.join(path)
                };
                let relative = absolute
                    .strip_prefix(root)
                    .map(|p| p.to_path_buf())
                    .unwrap_or(absolute);
                cmd.arg(relative);
            }
        }

        cmd.current_dir(root);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let child = cmd
            .spawn()
            .with_context(|| "failed to spawn ripgrep; is rg installed and on PATH?")?;

        let mut guard = ChildGuard::new(child);
        let child_ref = guard.as_mut().context("child process unavailable")?;

        let stdout = child_ref
            .stdout
            .take()
            .context("ripgrep did not produce stdout pipe")?;
        let stderr = child_ref
            .stderr
            .take()
            .context("ripgrep did not produce stderr pipe")?;

        let mut reader = BufReader::new(stdout).lines();
        let mut matches = Vec::new();
        let max_matches = self.max_matches;

        let collect = async {
            while let Some(line) = reader.next_line().await? {
                if matches.len() >= max_matches {
                    break;
                }
                let mut line_bytes = line.as_bytes().to_vec();
                let parsed: RgMessage = match simd_json::from_slice(&mut line_bytes) {
                    Ok(msg) => msg,
                    Err(err) => {
                        tracing::warn!(error = %err, "failed to parse ripgrep json line");
                        continue;
                    }
                };
                if let RgMessage::Match { data } = parsed {
                    let path = PathBuf::from(data.path.text);
                    matches.push(RipgrepMatch {
                        path,
                        line_number: data.line_number,
                        lines: data.lines.text,
                        raw_json: line.clone(),
                    });
                }
            }

            let mut child = guard.take().context("child process already taken")?;
            let status = child.wait().await?;

            if !status.success() && status.code() != Some(1) {
                let mut stderr_reader = BufReader::new(stderr).lines();
                let mut stderr_output = String::new();
                while let Ok(Some(line)) = stderr_reader.next_line().await {
                    if !stderr_output.is_empty() {
                        stderr_output.push('\n');
                    }
                    stderr_output.push_str(&line);
                    if stderr_output.len() > 1024 {
                        stderr_output.push_str("\n... (truncated)");
                        break;
                    }
                }
                if stderr_output.is_empty() {
                    anyhow::bail!("ripgrep exited with status {}", status);
                } else {
                    anyhow::bail!(
                        "ripgrep exited with status {}: {}",
                        status,
                        stderr_output.trim()
                    );
                }
            }
            Result::<Vec<RipgrepMatch>>::Ok(matches)
        };

        timeout(self.timeout, collect)
            .await
            .with_context(|| "ripgrep invocation timed out")?
    }
}

#[derive(Clone, Debug)]
pub struct RipgrepMatch {
    pub path: PathBuf,
    pub line_number: usize,
    pub lines: String,
    pub raw_json: String,
}

fn run_embedded_search(
    matcher: RegexMatcher,
    builder: SearcherBuilder,
    root: PathBuf,
    paths: Vec<PathBuf>,
    threads: usize,
    max_matches: usize,
    max_columns: usize,
    deadline: Instant,
) -> Result<Vec<RipgrepMatch>> {
    let matcher = Arc::new(matcher);
    let searcher_builder = Arc::new(builder);
    let results: Arc<Mutex<Vec<RipgrepMatch>>> = Arc::new(Mutex::new(Vec::new()));
    let timed_out = Arc::new(AtomicBool::new(false));
    let global_count = Arc::new(AtomicUsize::new(0));

    if threads <= 1 || paths.len() <= 1 {
        for path in paths {
            search_file(
                &path,
                &root,
                &matcher,
                &searcher_builder,
                &results,
                &timed_out,
                &global_count,
                max_matches,
                max_columns,
                deadline,
            );
            if timed_out.load(Ordering::Relaxed)
                || global_count.load(Ordering::Relaxed) >= max_matches
            {
                break;
            }
        }
    } else {
        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .context("failed to create rayon thread pool")?;

        pool.install(|| {
            paths.par_iter().for_each(|path| {
                search_file(
                    path,
                    &root,
                    &matcher,
                    &searcher_builder,
                    &results,
                    &timed_out,
                    &global_count,
                    max_matches,
                    max_columns,
                    deadline,
                );
            });
        });
    }

    if timed_out.load(Ordering::Relaxed) {
        anyhow::bail!("embedded ripgrep search timed out");
    }

    let mut matches = results.lock().unwrap().clone();
    matches.sort_by(|a, b| match a.path.cmp(&b.path) {
        std::cmp::Ordering::Equal => a.line_number.cmp(&b.line_number),
        other => other,
    });
    if matches.len() > max_matches {
        matches.truncate(max_matches);
    }
    Ok(matches)
}

fn search_file(
    path: &Path,
    root: &Path,
    matcher: &Arc<RegexMatcher>,
    builder: &Arc<SearcherBuilder>,
    results: &Arc<Mutex<Vec<RipgrepMatch>>>,
    timed_out: &Arc<AtomicBool>,
    global_count: &Arc<AtomicUsize>,
    max_matches: usize,
    max_columns: usize,
    deadline: Instant,
) {
    if should_stop(timed_out, global_count, max_matches, deadline) {
        return;
    }

    let matcher = &**matcher;
    let path_for_result = normalize_path(root, path);
    let path_str = path_for_result.to_string_lossy().to_string();
    let mut searcher = builder.as_ref().clone().build();
    let mut sink = MatchSink::new(
        path_for_result,
        path_str,
        max_columns,
        max_matches,
        deadline,
        Arc::clone(timed_out),
        Arc::clone(global_count),
    );

    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) => {
            tracing::debug!(
                error = %err,
                path = %path.display(),
                "failed to open file for search"
            );
            return;
        }
    };

    let reader = DeadlineReader::new(file, deadline, Arc::clone(timed_out));

    if let Err(err) = searcher.search_reader(matcher, reader, &mut sink) {
        if err.kind() != io::ErrorKind::TimedOut {
            tracing::debug!(error = %err, path = %path.display(), "search failed");
        }
    }

    if sink.has_timed_out() || timed_out.load(Ordering::Relaxed) {
        timed_out.store(true, Ordering::Relaxed);
        return;
    }

    if sink.has_matches() {
        let mut guard = results.lock().unwrap();
        guard.extend(sink.into_inner());
    }
}

fn should_stop(
    timed_out: &Arc<AtomicBool>,
    global_count: &Arc<AtomicUsize>,
    max_matches: usize,
    deadline: Instant,
) -> bool {
    if timed_out.load(Ordering::Relaxed) {
        return true;
    }
    if Instant::now() >= deadline {
        timed_out.store(true, Ordering::Relaxed);
        return true;
    }
    global_count.load(Ordering::Relaxed) >= max_matches
}

fn normalize_path(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).map_or_else(
        |_| path.to_path_buf(),
        |p| {
            if p.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                p.to_path_buf()
            }
        },
    )
}

fn format_line(bytes: &[u8], max_columns: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    if max_columns == 0 {
        return text.to_string();
    }

    let mut truncated = String::with_capacity(text.len());
    let mut count = 0;
    for ch in text.chars() {
        if count >= max_columns {
            break;
        }
        truncated.push(ch);
        count += 1;
    }
    truncated
}

fn build_raw_json(path: &str, line_number: usize, lines: &str) -> String {
    json!({
        "type": "match",
        "data": {
            "path": { "text": path },
            "lines": { "text": lines },
            "line_number": line_number,
        }
    })
    .to_string()
}

struct MatchSink {
    path: PathBuf,
    path_str: String,
    matches: Vec<RipgrepMatch>,
    max_columns: usize,
    max_matches: usize,
    deadline: Instant,
    timed_out: Arc<AtomicBool>,
    global_count: Arc<AtomicUsize>,
}

impl MatchSink {
    fn new(
        path: PathBuf,
        path_str: String,
        max_columns: usize,
        max_matches: usize,
        deadline: Instant,
        timed_out: Arc<AtomicBool>,
        global_count: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            path,
            path_str,
            matches: Vec::new(),
            max_columns,
            max_matches,
            deadline,
            timed_out,
            global_count,
        }
    }

    fn has_matches(&self) -> bool {
        !self.matches.is_empty()
    }

    fn has_timed_out(&self) -> bool {
        self.timed_out.load(Ordering::Relaxed)
    }

    fn into_inner(self) -> Vec<RipgrepMatch> {
        self.matches
    }

    fn reserve_slot(&self) -> bool {
        self.global_count
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                if count >= self.max_matches {
                    None
                } else {
                    Some(count + 1)
                }
            })
            .is_ok()
    }

    fn should_continue(&self) -> bool {
        if Instant::now() >= self.deadline {
            self.timed_out.store(true, Ordering::Relaxed);
            return false;
        }
        if self.timed_out.load(Ordering::Relaxed) {
            return false;
        }
        self.global_count.load(Ordering::Relaxed) < self.max_matches
    }
}

impl Sink for MatchSink {
    type Error = io::Error;

    fn matched(
        &mut self,
        _searcher: &Searcher,
        mat: &SinkMatch<'_>,
    ) -> std::result::Result<bool, Self::Error> {
        if !self.should_continue() {
            return Ok(false);
        }

        if !self.reserve_slot() {
            return Ok(false);
        }

        let line_number = mat.line_number().unwrap_or(0) as usize;
        let lines = format_line(mat.bytes(), self.max_columns);
        let raw_json = build_raw_json(&self.path_str, line_number, &lines);

        self.matches.push(RipgrepMatch {
            path: self.path.clone(),
            line_number,
            lines,
            raw_json,
        });

        Ok(self.should_continue())
    }

    fn context(
        &mut self,
        _searcher: &Searcher,
        _context: &SinkContext<'_>,
    ) -> std::result::Result<bool, Self::Error> {
        Ok(self.should_continue())
    }
}

struct DeadlineReader<R> {
    inner: R,
    deadline: Instant,
    timed_out: Arc<AtomicBool>,
}

impl<R> DeadlineReader<R> {
    fn new(inner: R, deadline: Instant, timed_out: Arc<AtomicBool>) -> Self {
        Self {
            inner,
            deadline,
            timed_out,
        }
    }
}

impl<R: Read> Read for DeadlineReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if Instant::now() >= self.deadline {
            self.timed_out.store(true, Ordering::Relaxed);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "search deadline reached",
            ));
        }
        self.inner.read(buf)
    }
}
