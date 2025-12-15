use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use super::facts::FileFacts;
use super::imports::ImportKind;

#[derive(Clone, Debug)]
pub struct ImportEdge {
    pub importer: String,
    pub imported: String,
    pub symbols: Vec<String>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct ImportGraph {
    edges: Vec<ImportEdge>,
    outgoing: HashMap<String, Vec<usize>>,
    incoming: HashMap<String, Vec<usize>>,
    exports: HashMap<String, Vec<String>>,
    wildcard_reexports: HashMap<String, Vec<String>>,
    files: HashSet<String>,
}

impl ImportGraph {
    pub fn from_facts(files: &[FileFacts]) -> Self {
        let file_set: HashSet<String> = files.iter().map(|f| normalize_rel_path(&f.file)).collect();

        let mut edges: Vec<ImportEdge> = Vec::new();
        let mut outgoing: HashMap<String, Vec<usize>> = HashMap::new();
        let mut incoming: HashMap<String, Vec<usize>> = HashMap::new();
        let mut exports: HashMap<String, Vec<String>> = HashMap::new();
        let mut wildcard_reexports: HashMap<String, Vec<String>> = HashMap::new();

        for file in files {
            let importer = normalize_rel_path(&file.file);

            // Best-effort export index: treat definitions as export points, then layer in language
            // export facts and explicit re-exports.
            for def in &file.facts.definitions {
                exports
                    .entry(def.symbol.clone())
                    .or_default()
                    .push(importer.clone());
            }
            for export in &file.facts.exports {
                exports
                    .entry(export.symbol.clone())
                    .or_default()
                    .push(importer.clone());
            }

            for import in &file.facts.imports {
                let imported = resolve_import_source(&importer, &import.source, &file_set);

                let idx = edges.len();
                edges.push(ImportEdge {
                    importer: importer.clone(),
                    imported: imported.clone(),
                    symbols: import.symbols.clone(),
                    line: import.line,
                });
                outgoing.entry(importer.clone()).or_default().push(idx);
                incoming.entry(imported.clone()).or_default().push(idx);

                if import.kind == ImportKind::Reexport {
                    if import.symbols.is_empty() {
                        wildcard_reexports
                            .entry(importer.clone())
                            .or_default()
                            .push(imported.clone());
                    } else {
                        for sym in &import.symbols {
                            let exported_name = import.alias.as_deref().unwrap_or(sym);
                            exports
                                .entry(exported_name.to_string())
                                .or_default()
                                .push(importer.clone());
                        }
                    }
                }
            }
        }

        dedup_map_values(&mut exports);
        dedup_map_values(&mut outgoing);
        dedup_map_values(&mut incoming);
        dedup_map_values(&mut wildcard_reexports);

        Self {
            edges,
            outgoing,
            incoming,
            exports,
            wildcard_reexports,
            files: file_set,
        }
    }

    pub fn imports_of(&self, file: &str) -> Vec<&ImportEdge> {
        let key = normalize_rel_path(file);
        self.outgoing
            .get(&key)
            .into_iter()
            .flat_map(|idxs| idxs.iter().copied())
            .filter_map(|idx| self.edges.get(idx))
            .collect()
    }

    pub fn importers_of(&self, file: &str) -> Vec<&ImportEdge> {
        let key = normalize_rel_path(file);
        self.incoming
            .get(&key)
            .into_iter()
            .flat_map(|idxs| idxs.iter().copied())
            .filter_map(|idx| self.edges.get(idx))
            .collect()
    }

    pub fn find_export(&self, symbol: &str) -> Vec<&str> {
        let symbol = symbol.trim();
        if symbol.is_empty() {
            return Vec::new();
        }

        let mut exported_from: HashSet<String> = self
            .exports
            .get(symbol)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();

        let mut changed = true;
        while changed {
            changed = false;
            for (exporter, sources) in &self.wildcard_reexports {
                if exported_from.contains(exporter) {
                    continue;
                }
                if sources.iter().any(|src| exported_from.contains(src)) {
                    changed = true;
                    exported_from.insert(exporter.clone());
                }
            }
        }

        let mut out: Vec<&str> = Vec::new();
        for path in exported_from {
            if let Some(existing) = self.files.get(&path) {
                out.push(existing.as_str());
            }
        }
        out.sort();
        out
    }

    pub fn transitive_imports(&self, file: &str, depth: usize) -> Vec<String> {
        self.transitive(file, depth, Direction::Outgoing)
    }

    pub fn transitive_importers(&self, file: &str, depth: usize) -> Vec<String> {
        self.transitive(file, depth, Direction::Incoming)
    }

    fn transitive(&self, file: &str, depth: usize, direction: Direction) -> Vec<String> {
        let depth = depth.max(1);
        let start = normalize_rel_path(file);
        let mut out: Vec<String> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(start.clone());

        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        queue.push_back((start, 0));

        while let Some((current, d)) = queue.pop_front() {
            if d >= depth {
                continue;
            }

            let idxs: Vec<usize> = match direction {
                Direction::Outgoing => self.outgoing.get(&current).cloned().unwrap_or_default(),
                Direction::Incoming => self.incoming.get(&current).cloned().unwrap_or_default(),
            };

            for idx in idxs {
                let Some(edge) = self.edges.get(idx) else {
                    continue;
                };
                let next = match direction {
                    Direction::Outgoing => edge.imported.clone(),
                    Direction::Incoming => edge.importer.clone(),
                };
                if !visited.insert(next.clone()) {
                    continue;
                }
                out.push(next.clone());
                queue.push_back((next, d + 1));
            }
        }

        out
    }

    pub fn is_known_file(&self, file: &str) -> bool {
        self.files.contains(&normalize_rel_path(file))
    }
}

#[derive(Copy, Clone, Debug)]
enum Direction {
    Outgoing,
    Incoming,
}

fn resolve_import_source(importer: &str, source: &str, files: &HashSet<String>) -> String {
    let source = source.trim();
    if source.is_empty() {
        return String::new();
    }

    // TypeScript/JS relative imports: ./, ../
    if source.starts_with("./") || source.starts_with("../") {
        return resolve_relative_ts(importer, source, files);
    }

    // Rust "mod foo;" facts are emitted as bare identifiers; resolve them like modules.
    if is_rust_file(importer) && is_bare_ident(source) {
        return resolve_relative_rust_mod(importer, source, files);
    }

    source.to_string()
}

fn resolve_relative_ts(importer: &str, source: &str, files: &HashSet<String>) -> String {
    let base_dir = Path::new(importer).parent().unwrap_or(Path::new(""));
    let joined = normalize_path(base_dir.join(source));
    let joined_str = joined.to_string_lossy().replace('\\', "/");

    if has_extension_like(&joined_str) {
        return pick_existing(&joined_str, files).unwrap_or(joined_str);
    }

    let candidates = ts_path_candidates(&joined_str);
    for candidate in candidates {
        if files.contains(&candidate) {
            return candidate;
        }
    }

    joined_str
}

fn resolve_relative_rust_mod(importer: &str, module: &str, files: &HashSet<String>) -> String {
    let base_dir = Path::new(importer).parent().unwrap_or(Path::new(""));
    let root = normalize_path(base_dir.join(module));
    let base = root.to_string_lossy().replace('\\', "/");

    for candidate in [format!("{base}.rs"), format!("{base}/mod.rs")] {
        if files.contains(&candidate) {
            return candidate;
        }
    }

    format!("{base}.rs")
}

fn ts_path_candidates(base: &str) -> Vec<String> {
    let mut out = Vec::new();
    let exts = ["ts", "tsx", "js", "jsx", "d.ts"];
    for ext in exts {
        if ext.contains('.') {
            out.push(format!("{base}.{ext}"));
            out.push(format!("{base}/index.{ext}"));
        } else {
            out.push(format!("{base}.{ext}"));
            out.push(format!("{base}/index.{ext}"));
        }
    }
    out
}

fn pick_existing(path: &str, files: &HashSet<String>) -> Option<String> {
    let normalized = normalize_rel_path(path);
    if files.contains(&normalized) {
        return Some(normalized);
    }
    None
}

fn normalize_rel_path(path: &str) -> String {
    let trimmed = path.trim().trim_start_matches("./");
    normalize_path(PathBuf::from(trimmed))
        .to_string_lossy()
        .replace('\\', "/")
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn is_rust_file(file: &str) -> bool {
    file.ends_with(".rs")
}

fn is_bare_ident(raw: &str) -> bool {
    raw.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn has_extension_like(path: &str) -> bool {
    let file = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    file.contains('.')
}

fn dedup_map_values<T: Clone + Eq + std::hash::Hash>(map: &mut HashMap<String, Vec<T>>) {
    for values in map.values_mut() {
        let mut seen: HashSet<T> = HashSet::new();
        values.retain(|v| seen.insert(v.clone()));
    }
}
