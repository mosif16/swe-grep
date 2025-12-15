use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::facts::FileFacts;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct CallEdge {
    pub caller: String,
    pub callee: String,
    pub file: String,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct CallGraph {
    edges: Vec<CallEdge>,
    outgoing: HashMap<String, Vec<usize>>,
    incoming: HashMap<String, Vec<usize>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CallGraphIndex {
    outgoing: HashMap<String, Vec<CallEdge>>,
    incoming: HashMap<String, Vec<CallEdge>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CallTreeNode {
    pub symbol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<CallTreeNode>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CallGraphView {
    pub root: String,
    pub direction: String,
    pub depth: usize,
    pub nodes: Vec<CallTreeNode>,
    pub truncated: bool,
}

impl CallGraph {
    pub fn from_facts(files: &[FileFacts]) -> Self {
        let mut edges: Vec<CallEdge> = Vec::new();
        for file in files {
            for call in &file.facts.calls {
                edges.push(CallEdge {
                    caller: call.caller.clone(),
                    callee: call.callee.clone(),
                    file: file.file.clone(),
                    line: call.line,
                });
            }
        }

        // Build indexes for both qualified and unqualified lookups
        let mut outgoing: HashMap<String, Vec<usize>> = HashMap::new();
        let mut incoming: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, edge) in edges.iter().enumerate() {
            // Index by full qualified name (Type::method)
            let caller_full = normalize_qualified(&edge.caller);
            let callee_full = normalize_qualified(&edge.callee);
            outgoing.entry(caller_full.clone()).or_default().push(idx);
            incoming.entry(callee_full.clone()).or_default().push(idx);

            // Also index by simple name for unqualified queries
            let caller_simple = extract_simple_name(&edge.caller);
            let callee_simple = extract_simple_name(&edge.callee);
            if caller_simple != caller_full {
                outgoing.entry(caller_simple).or_default().push(idx);
            }
            if callee_simple != callee_full {
                incoming.entry(callee_simple).or_default().push(idx);
            }
        }

        Self {
            edges,
            outgoing,
            incoming,
        }
    }

    pub fn callers_view(&self, target: &str, depth: usize, max_nodes: usize) -> CallGraphView {
        self.build_view(target, depth, max_nodes, Direction::Callers)
    }

    pub fn callees_view(&self, target: &str, depth: usize, max_nodes: usize) -> CallGraphView {
        self.build_view(target, depth, max_nodes, Direction::Callees)
    }

    pub fn caller_edges_for(&self, target: &str) -> Vec<CallEdge> {
        let key = normalize_symbol(target);
        self.incoming
            .get(&key)
            .into_iter()
            .flat_map(|idxs| idxs.iter().copied())
            .filter_map(|idx| self.edges.get(idx).cloned())
            .collect()
    }

    pub fn callee_edges_for(&self, target: &str) -> Vec<CallEdge> {
        let key = normalize_symbol(target);
        self.outgoing
            .get(&key)
            .into_iter()
            .flat_map(|idxs| idxs.iter().copied())
            .filter_map(|idx| self.edges.get(idx).cloned())
            .collect()
    }

    fn build_view(
        &self,
        target: &str,
        depth: usize,
        max_nodes: usize,
        direction: Direction,
    ) -> CallGraphView {
        let depth = depth.max(1);
        let max_nodes = max_nodes.max(1);

        let mut truncated = false;
        let mut seen: HashSet<String> = HashSet::new();
        let mut budget = max_nodes;
        let root = normalize_symbol(target);
        seen.insert(root.clone());

        let nodes = self.expand(
            &root,
            depth,
            &mut seen,
            &mut budget,
            &mut truncated,
            direction,
        );

        CallGraphView {
            root,
            direction: direction.as_str().to_string(),
            depth,
            nodes,
            truncated,
        }
    }

    fn expand(
        &self,
        current: &str,
        depth: usize,
        seen: &mut HashSet<String>,
        budget: &mut usize,
        truncated: &mut bool,
        direction: Direction,
    ) -> Vec<CallTreeNode> {
        if depth == 0 || *budget == 0 {
            return Vec::new();
        }

        let edge_idxs: Vec<usize> = match direction {
            Direction::Callers => self.incoming.get(current).cloned().unwrap_or_default(),
            Direction::Callees => self.outgoing.get(current).cloned().unwrap_or_default(),
        };

        let mut children: Vec<CallTreeNode> = Vec::new();
        for idx in edge_idxs {
            if *budget == 0 {
                *truncated = true;
                break;
            }
            let Some(edge) = self.edges.get(idx) else {
                continue;
            };

            let next_symbol = match direction {
                Direction::Callers => normalize_symbol(&edge.caller),
                Direction::Callees => normalize_symbol(&edge.callee),
            };

            if !seen.insert(next_symbol.clone()) {
                continue;
            }

            *budget = budget.saturating_sub(1);

            let grand = self.expand(
                &next_symbol,
                depth.saturating_sub(1),
                seen,
                budget,
                truncated,
                direction,
            );

            children.push(CallTreeNode {
                symbol: next_symbol,
                file: Some(edge.file.clone()),
                line: Some(edge.line),
                children: grand,
            });
        }

        children
    }
}

#[derive(Copy, Clone, Debug)]
enum Direction {
    Callers,
    Callees,
}

impl Direction {
    fn as_str(&self) -> &'static str {
        match self {
            Direction::Callers => "callers",
            Direction::Callees => "callees",
        }
    }
}

/// Normalize a symbol for lookup, preserving qualification if present
pub fn normalize_symbol(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // If the query contains ::, use qualified lookup
    if trimmed.contains("::") {
        return normalize_qualified(trimmed);
    }

    // Otherwise, use simple name for broader matching
    extract_simple_name(trimmed)
}

/// Preserve the full qualified path (Type::method)
fn normalize_qualified(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Clean up but preserve :: separators
    let mut result = String::new();
    let mut prev_was_sep = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
            result.push(ch);
            prev_was_sep = false;
        } else if ch == ':' && !prev_was_sep {
            result.push(ch);
            prev_was_sep = true;
        } else if ch == ':' && prev_was_sep {
            result.push(ch);
            prev_was_sep = false;
        } else {
            prev_was_sep = false;
        }
    }

    // Clean up trailing/leading colons
    result.trim_matches(':').to_string()
}

/// Extract just the simple name (last component after ::)
fn extract_simple_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Split by :: and take the last non-empty component
    if let Some(last) = trimmed.rsplit("::").next() {
        let cleaned: String = last
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '$')
            .collect();
        if !cleaned.is_empty() {
            return cleaned;
        }
    }

    // Fallback: extract last identifier
    let mut last = String::new();
    for token in trimmed.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')) {
        if !token.is_empty() {
            last = token.to_string();
        }
    }
    if last.is_empty() {
        trimmed.to_string()
    } else {
        last
    }
}

impl CallGraphIndex {
    pub fn caller_edges_for(&self, target: &str) -> Vec<CallEdge> {
        let key = normalize_symbol(target);
        self.incoming.get(&key).cloned().unwrap_or_default()
    }

    pub fn callee_edges_for(&self, target: &str) -> Vec<CallEdge> {
        let key = normalize_symbol(target);
        self.outgoing.get(&key).cloned().unwrap_or_default()
    }

    pub fn add_edges(&mut self, edges: impl IntoIterator<Item = CallEdge>) {
        for edge in edges {
            self.add_edge(edge);
        }
    }

    pub fn remove_edges(&mut self, edges: impl IntoIterator<Item = CallEdge>) {
        for edge in edges {
            self.remove_edge(&edge);
        }
    }

    fn add_edge(&mut self, edge: CallEdge) {
        let caller_full = normalize_qualified(&edge.caller);
        let callee_full = normalize_qualified(&edge.callee);

        self.outgoing
            .entry(caller_full.clone())
            .or_default()
            .push(edge.clone());
        self.incoming
            .entry(callee_full.clone())
            .or_default()
            .push(edge.clone());

        let caller_simple = extract_simple_name(&edge.caller);
        let callee_simple = extract_simple_name(&edge.callee);

        if caller_simple != caller_full {
            self.outgoing
                .entry(caller_simple)
                .or_default()
                .push(edge.clone());
        }
        if callee_simple != callee_full {
            self.incoming.entry(callee_simple).or_default().push(edge);
        }
    }

    fn remove_edge(&mut self, edge: &CallEdge) {
        let caller_full = normalize_qualified(&edge.caller);
        let callee_full = normalize_qualified(&edge.callee);
        let caller_simple = extract_simple_name(&edge.caller);
        let callee_simple = extract_simple_name(&edge.callee);

        remove_index_edge(&mut self.outgoing, &caller_full, edge);
        remove_index_edge(&mut self.incoming, &callee_full, edge);

        if caller_simple != caller_full {
            remove_index_edge(&mut self.outgoing, &caller_simple, edge);
        }
        if callee_simple != callee_full {
            remove_index_edge(&mut self.incoming, &callee_simple, edge);
        }
    }
}

fn remove_index_edge(map: &mut HashMap<String, Vec<CallEdge>>, key: &str, edge: &CallEdge) {
    let remove_empty = if let Some(v) = map.get_mut(key) {
        v.retain(|candidate| candidate != edge);
        v.is_empty()
    } else {
        false
    };
    if remove_empty {
        map.remove(key);
    }
}
