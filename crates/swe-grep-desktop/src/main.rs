use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui;
use swe_grep::cli::{SearchArgs, ToolArgs};
use swe_grep::search;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Welcome,
    Search,
}

impl Default for View {
    fn default() -> Self {
        Self::Welcome
    }
}

#[derive(Clone, Debug)]
struct HitRow {
    symbol: String,
    file: String,
    line: usize,
    origin: String,
    score: f32,
    snippet: String,
    context: String,
    body: String,
    body_truncated: bool,
    body_retrieved: bool,
    warnings: Vec<String>,
}

#[derive(Default)]
struct DesktopApp {
    view: View,
    repo_path: String,
    symbols: String,

    auto_run: bool,
    auto_run_debounce_ms: u64,
    auto_run_pending_since: Option<Instant>,
    last_fingerprint: u64,
    auto_run_on_start: bool,

    text: bool,
    regex: bool,
    wildcard: bool,
    string: bool,
    ignore_case: bool,
    include_strings: bool,
    code_only: bool,
    body: bool,
    body_limit: String,
    no_truncate: bool,

    max_matches: usize,
    top_hits: usize,
    page_size: usize,
    page: usize,

    status: String,
    rows: Vec<HitRow>,
    selected: Option<usize>,

    in_flight: bool,
    rx: Option<mpsc::Receiver<Result<Vec<HitRow>>>>,
}

impl DesktopApp {
    fn new() -> Self {
        swe_grep::telemetry::silence_tracing();
        let mut app = Self {
            view: View::Welcome,
            repo_path: std::env::current_dir()
                .ok()
                .unwrap_or_else(|| PathBuf::from("."))
                .to_string_lossy()
                .to_string(),
            symbols: "login_user".to_string(),
            auto_run: true,
            auto_run_debounce_ms: 400,
            auto_run_pending_since: None,
            last_fingerprint: 0,
            auto_run_on_start: false,
            max_matches: 200,
            top_hits: 200,
            page_size: 50,
            ..Self::default()
        };
        app.last_fingerprint = app.fingerprint();
        app
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.repo_path.trim().hash(&mut hasher);
        self.symbols.trim().hash(&mut hasher);
        self.text.hash(&mut hasher);
        self.regex.hash(&mut hasher);
        self.wildcard.hash(&mut hasher);
        self.string.hash(&mut hasher);
        self.ignore_case.hash(&mut hasher);
        self.include_strings.hash(&mut hasher);
        self.code_only.hash(&mut hasher);
        self.body.hash(&mut hasher);
        self.body_limit.trim().hash(&mut hasher);
        self.no_truncate.hash(&mut hasher);
        self.max_matches.hash(&mut hasher);
        self.top_hits.hash(&mut hasher);
        hasher.finish()
    }

    fn kick_search(&mut self) {
        if self.in_flight {
            return;
        }

        let repo_path = self.repo_path.trim().to_string();
        let symbols_raw = self.symbols.clone();

        let text = self.text;
        let regex = self.regex;
        let wildcard = self.wildcard;
        let string = self.string;
        let ignore_case = self.ignore_case;
        let include_strings = self.include_strings;
        let code_only = self.code_only;
        let body = self.body;
        let no_truncate = self.no_truncate;
        let body_limit = self.body_limit.trim().to_string();
        let max_matches = self.max_matches;
        let top_hits = self.top_hits;

        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.in_flight = true;
        self.status = "Searching...".to_string();

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new();
            let result: Result<Vec<HitRow>> = match runtime {
                Ok(runtime) => runtime.block_on(async move {
                    let repo_root = PathBuf::from(repo_path);
                    let symbols: Vec<String> = symbols_raw
                        .split([',', '\n'])
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .collect();
                    if symbols.is_empty() {
                        anyhow::bail!("at least one symbol is required");
                    }

                    let parsed_body_limit = if body_limit.is_empty() {
                        None
                    } else {
                        Some(
                            body_limit
                                .parse::<usize>()
                                .map_err(|_| anyhow::anyhow!("invalid --body-limit bytes value"))?,
                        )
                    };

                    let mut all_rows: Vec<HitRow> = Vec::new();
                    for symbol in symbols {
                        let args = SearchArgs {
                            symbol: symbol.clone(),
                            text,
                            concept: None,
                            concept_expand: false,
                            regex,
                            literal_regex: false,
                            wildcard,
                            attribute: false,
                            string,
                            verbose_ast: false,
                            include_strings,
                            code_only,
                            ignore_case,
                            definitions_only: false,
                            usages_only: false,
                            path: Some(repo_root.clone()),
                            scope_file: None,
                            path_glob: Vec::new(),
                            exclude_patterns: Vec::new(),
                            exclude_tests: false,
                            language: None,
                            mode: None,
                            kind: None,
                            timeout_secs: 3,
                            max_matches,
                            top_hits,
                            max_columns: 200,
                            concurrency: 8,
                            tools: ToolArgs {
                                enable_index: false,
                                enable_rga: false,
                                use_fd: true,
                                use_ast_grep: true,
                            },
                            context: None,
                            context_before: 0,
                            context_after: 0,
                            body,
                            body_limit: parsed_body_limit,
                            no_truncate,
                            git_modified: false,
                            git_since: None,
                            git_info: false,
                            index_dir: None,
                            cache_dir: None,
                            log_dir: None,
                        };

                        let summary = search::execute(args).await?;
                        for hit in summary.top_hits {
                            let body = hit.body.unwrap_or_default();
                            all_rows.push(HitRow {
                                symbol: symbol.clone(),
                                file: hit.path,
                                line: hit.line,
                                origin: hit.origin_label,
                                score: hit.score,
                                snippet: hit.snippet.unwrap_or_default(),
                                context: hit.expanded_snippet.unwrap_or_default(),
                                body,
                                body_truncated: hit.body_truncated,
                                body_retrieved: hit.body_retrieved,
                                warnings: summary.warnings.clone(),
                            });
                        }
                    }

                    all_rows.sort_by(|a, b| {
                        b.score
                            .partial_cmp(&a.score)
                            .unwrap_or(std::cmp::Ordering::Equal)
                            .then_with(|| a.file.cmp(&b.file))
                            .then_with(|| a.line.cmp(&b.line))
                    });

                    Ok(all_rows)
                }),
                Err(err) => Err(err.into()),
            };

            let _ = tx.send(result);
        });
    }

    fn poll_results(&mut self) {
        let Some(rx) = &self.rx else { return };
        match rx.try_recv() {
            Ok(result) => {
                self.in_flight = false;
                self.rx = None;
                match result {
                    Ok(rows) => {
                        self.page = 0;
                        self.selected = None;
                        self.status = format!("{} hits", rows.len());
                        self.rows = rows;
                    }
                    Err(err) => {
                        self.status = format!("Error: {err}");
                        self.rows.clear();
                        self.selected = None;
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.in_flight = false;
                self.rx = None;
                self.status = "Search worker disconnected".to_string();
            }
        }
    }

    fn load_fixture_defaults(&mut self) {
        self.repo_path = "fixtures/multi_lang".to_string();
        self.symbols = "login_user,compute_checksum".to_string();
        self.body = true;
        self.no_truncate = false;
        self.body_limit = "2048".to_string();
        self.max_matches = 200;
        self.top_hits = 200;
        self.page_size = 50;
        self.page = 0;
        self.selected = None;
        self.last_fingerprint = self.fingerprint();
        self.auto_run_pending_since = Some(Instant::now());
    }

    fn page_bounds(&self) -> (usize, usize) {
        let total = self.rows.len();
        if total == 0 {
            return (0, 0);
        }
        let page_size = self.page_size.max(1);
        let start = self.page.saturating_mul(page_size).min(total);
        let end = (start + page_size).min(total);
        (start, end)
    }
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.style().visuals.dark_mode == false {
            ctx.set_visuals(egui::Visuals::dark());
        }

        self.poll_results();

        if self.view == View::Search && self.auto_run_on_start {
            self.auto_run_on_start = false;
            if !self.in_flight {
                self.kick_search();
            }
        }

        if self.view == View::Search {
            let fingerprint = self.fingerprint();
            if fingerprint != self.last_fingerprint {
                self.last_fingerprint = fingerprint;
                if self.auto_run {
                    self.auto_run_pending_since = Some(Instant::now());
                }
            }
            if self.auto_run {
                if let Some(since) = self.auto_run_pending_since {
                    if !self.in_flight
                        && since.elapsed() >= Duration::from_millis(self.auto_run_debounce_ms)
                    {
                        self.auto_run_pending_since = None;
                        self.kick_search();
                    }
                }
            } else {
                self.auto_run_pending_since = None;
            }
        }

        if self.view == View::Welcome {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(32.0);
                    ui.heading(egui::RichText::new("SWE-Grep Desktop").size(34.0).strong());
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Fast, local code search with previews — packaged as a desktop app.")
                            .size(16.0)
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(24.0);

                    egui::Frame::none()
                        .fill(ui.visuals().widgets.inactive.bg_fill)
                        .rounding(egui::Rounding::same(14.0))
                        .inner_margin(egui::Margin::same(16.0))
                        .show(ui, |ui| {
                            ui.set_width(640.0);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Repo").strong());
                                ui.add(egui::TextEdit::singleline(&mut self.repo_path).desired_width(f32::INFINITY));
                            });
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Symbols").strong());
                                ui.add(egui::TextEdit::singleline(&mut self.symbols).desired_width(f32::INFINITY));
                            });
                            ui.add_space(12.0);

                            ui.horizontal(|ui| {
                                let start = ui
                                    .add_sized(
                                        [140.0, 38.0],
                                        egui::Button::new(egui::RichText::new("Start").size(16.0).strong()),
                                    )
                                    .clicked();
                                if ui
                                    .add_sized(
                                        [140.0, 38.0],
                                        egui::Button::new(egui::RichText::new("Use Fixtures").size(16.0)),
                                    )
                                    .clicked()
                                {
                                    self.load_fixture_defaults();
                                }

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.checkbox(&mut self.auto_run, "Auto-run");
                                });

                                if start {
                                    self.view = View::Search;
                                    self.auto_run_on_start = true;
                                    self.auto_run_pending_since = Some(Instant::now());
                                }
                            });
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new("Tip: separate multiple symbols with commas, or paste multiple lines.")
                                    .color(ui.visuals().weak_text_color()),
                            );
                        });
                });
            });
            return;
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new("SWE-Grep").size(18.0));
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Repo").color(ui.visuals().weak_text_color()));
                ui.add(egui::TextEdit::singleline(&mut self.repo_path).desired_width(260.0));
                ui.label(egui::RichText::new("Symbols").color(ui.visuals().weak_text_color()));
                let symbols_resp =
                    ui.add(egui::TextEdit::singleline(&mut self.symbols).desired_width(260.0));
                if symbols_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.kick_search();
                }
                if ui
                    .add_enabled(!self.in_flight, egui::Button::new("Search"))
                    .clicked()
                {
                    self.kick_search();
                }
                if ui.add(egui::Button::new("Home")).clicked() {
                    self.view = View::Welcome;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(&self.status).color(ui.visuals().weak_text_color()),
                    );
                });
            });

            ui.horizontal(|ui| {
                ui.label("max-matches:");
                ui.add(egui::DragValue::new(&mut self.max_matches).range(1..=10_000));
                ui.label("top-hits:");
                ui.add(egui::DragValue::new(&mut self.top_hits).range(1..=10_000));
                ui.label("page-size:");
                ui.add(egui::DragValue::new(&mut self.page_size).range(1..=500));
                ui.separator();
                ui.checkbox(&mut self.auto_run, "Auto-run");
                ui.add_enabled(
                    self.auto_run,
                    egui::DragValue::new(&mut self.auto_run_debounce_ms).range(0..=2_000),
                );
                ui.label("ms");
                if ui
                    .add_enabled(!self.in_flight, egui::Button::new("Use Fixtures"))
                    .clicked()
                {
                    self.load_fixture_defaults();
                }
            });

            ui.collapsing("Advanced", |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.text, "--text");
                    ui.checkbox(&mut self.regex, "--regex");
                    ui.checkbox(&mut self.wildcard, "--wildcard");
                    ui.checkbox(&mut self.string, "--string");
                    ui.checkbox(&mut self.ignore_case, "--ignore-case");
                    ui.checkbox(&mut self.include_strings, "--include-strings");
                    ui.checkbox(&mut self.code_only, "--code-only");
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.body, "--body");
                    ui.checkbox(&mut self.no_truncate, "--no-truncate");
                    ui.label("--body-limit:");
                    ui.text_edit_singleline(&mut self.body_limit);
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let (start, end) = self.page_bounds();
            let total_pages = if self.rows.is_empty() {
                0
            } else {
                (self.rows.len().saturating_sub(1) / self.page_size.max(1)) + 1
            };

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.page > 0, egui::Button::new("Prev"))
                    .clicked()
                {
                    self.page = self.page.saturating_sub(1);
                    self.selected = None;
                }
                ui.label(format!("Page {} / {}", self.page + 1, total_pages.max(1)));
                if ui
                    .add_enabled(self.page + 1 < total_pages, egui::Button::new("Next"))
                    .clicked()
                {
                    self.page += 1;
                    self.selected = None;
                }
                ui.label(format!("Showing {}..{} of {}", start, end, self.rows.len()));
            });

            ui.separator();

            ui.columns(2, |cols| {
                cols[0].vertical(|ui| {
                    egui::Frame::none()
                        .fill(ui.visuals().widgets.inactive.bg_fill)
                        .rounding(egui::Rounding::same(10.0))
                        .inner_margin(egui::Margin::same(10.0))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(format!("Results ({} hits)", self.rows.len()))
                                    .strong(),
                            );
                            ui.add_space(6.0);
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                for (idx, row) in self.rows[start..end].iter().enumerate() {
                                    let absolute = start + idx;
                                    let selected = self.selected == Some(absolute);

                                    let title =
                                        format!("{}:{}  {}", row.file, row.line, row.symbol);
                                    let subtitle = format!("{:.2}  {}", row.score, row.origin);
                                    let snippet = row.snippet.lines().next().unwrap_or("").trim();

                                    let button = egui::SelectableLabel::new(
                                        selected,
                                        egui::RichText::new(title).monospace().strong(),
                                    );
                                    if ui.add(button).clicked() {
                                        self.selected = Some(absolute);
                                    }
                                    ui.label(
                                        egui::RichText::new(subtitle)
                                            .size(12.0)
                                            .color(ui.visuals().weak_text_color()),
                                    );
                                    if !snippet.is_empty() {
                                        ui.label(
                                            egui::RichText::new(snippet)
                                                .size(12.0)
                                                .color(egui::Color32::from_gray(160)),
                                        );
                                    }
                                    ui.add_space(10.0);
                                }
                            });
                        });
                });

                cols[1].vertical(|ui| {
                    egui::Frame::none()
                        .fill(ui.visuals().widgets.inactive.bg_fill)
                        .rounding(egui::Rounding::same(10.0))
                        .inner_margin(egui::Margin::same(10.0))
                        .show(ui, |ui| {
                            if let Some(index) = self.selected {
                                if let Some(hit) = self.rows.get(index) {
                                    ui.label(egui::RichText::new("Preview").strong());
                                    ui.add_space(6.0);
                                    ui.label(
                                        egui::RichText::new(format!("{}:{}", hit.file, hit.line))
                                            .monospace()
                                            .strong(),
                                    );
                                    ui.label(
                                egui::RichText::new(format!(
                                    "score {:.2} | {} | body_retrieved={} | body_bytes={}{}",
                                    hit.score,
                                    hit.origin,
                                    hit.body_retrieved,
                                    hit.body.len(),
                                    if hit.body_truncated {
                                        " (truncated)"
                                    } else {
                                        ""
                                    }
                                ))
                                .size(12.0)
                                .color(ui.visuals().weak_text_color()),
                            );
                                    if !hit.warnings.is_empty() {
                                        ui.separator();
                                        ui.label("warnings:");
                                        for w in &hit.warnings {
                                            ui.label(format!("- {w}"));
                                        }
                                    }
                                    ui.separator();
                                    ui.collapsing("snippet", |ui| {
                                        let mut text = hit.snippet.clone();
                                        ui.add(
                                            egui::TextEdit::multiline(&mut text)
                                                .desired_rows(6)
                                                .code_editor()
                                                .interactive(false),
                                        );
                                    });
                                    ui.collapsing("context", |ui| {
                                        let mut text = hit.context.clone();
                                        ui.add(
                                            egui::TextEdit::multiline(&mut text)
                                                .desired_rows(10)
                                                .code_editor()
                                                .interactive(false),
                                        );
                                    });
                                    ui.collapsing("body", |ui| {
                                        let mut text = hit.body.clone();
                                        ui.add(
                                            egui::TextEdit::multiline(&mut text)
                                                .desired_rows(20)
                                                .code_editor()
                                                .interactive(false),
                                        );
                                    });
                                }
                            } else {
                                ui.label(
                                    egui::RichText::new("Select a hit to preview.")
                                        .color(ui.visuals().weak_text_color()),
                                );
                            }
                        });
                });
            });
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "SWE-Grep Desktop",
        options,
        Box::new(|_cc| Ok(Box::new(DesktopApp::new()))),
    )
}
