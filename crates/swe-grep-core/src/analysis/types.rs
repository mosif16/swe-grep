use std::collections::{HashMap, HashSet};

use serde::Serialize;

use super::facts::FileFacts;

#[derive(Clone, Debug, Serialize)]
pub struct TypeRelationEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct TypeGraph {
    edges: Vec<TypeRelationEdge>,
    outgoing: HashMap<String, Vec<usize>>,
    incoming: HashMap<String, Vec<usize>>,
}

#[derive(Copy, Clone, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TypeDirection {
    Up,
    Down,
    Both,
}

#[derive(Clone, Debug, Serialize)]
pub struct TypeTreeNode {
    pub symbol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TypeTreeNode>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TypeGraphView {
    pub root: String,
    pub direction: TypeDirection,
    pub depth: usize,
    pub nodes: Vec<TypeTreeNode>,
    pub truncated: bool,
}

impl TypeGraph {
    pub fn from_facts(files: &[FileFacts]) -> Self {
        let mut edges = Vec::new();
        for file in files {
            for rel in &file.facts.relations {
                edges.push(TypeRelationEdge {
                    from: rel.from.clone(),
                    to: rel.to.clone(),
                    kind: rel.kind.clone(),
                    file: file.file.clone(),
                    line: rel.line,
                });
            }
        }

        let mut outgoing: HashMap<String, Vec<usize>> = HashMap::new();
        let mut incoming: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, edge) in edges.iter().enumerate() {
            outgoing
                .entry(edge.from.clone())
                .or_default()
                .push(idx);
            incoming.entry(edge.to.clone()).or_default().push(idx);
        }

        Self {
            edges,
            outgoing,
            incoming,
        }
    }

    pub fn view(
        &self,
        symbol: &str,
        direction: TypeDirection,
        depth: usize,
        max_nodes: usize,
    ) -> TypeGraphView {
        let depth = depth.max(1);
        let max_nodes = max_nodes.max(1);
        let root = symbol.trim().to_string();

        let mut seen: HashSet<String> = HashSet::new();
        let mut budget = max_nodes;
        let mut truncated = false;
        seen.insert(root.clone());

        let nodes = match direction {
            TypeDirection::Up => self.expand_up(&root, depth, &mut seen, &mut budget, &mut truncated),
            TypeDirection::Down => {
                self.expand_down(&root, depth, &mut seen, &mut budget, &mut truncated)
            }
            TypeDirection::Both => {
                let mut up = self.expand_up(&root, depth, &mut seen, &mut budget, &mut truncated);
                let mut down =
                    self.expand_down(&root, depth, &mut seen, &mut budget, &mut truncated);
                up.append(&mut down);
                up
            }
        };

        TypeGraphView {
            root,
            direction,
            depth,
            nodes,
            truncated,
        }
    }

    fn expand_up(
        &self,
        current: &str,
        depth: usize,
        seen: &mut HashSet<String>,
        budget: &mut usize,
        truncated: &mut bool,
    ) -> Vec<TypeTreeNode> {
        if depth == 0 || *budget == 0 {
            return Vec::new();
        }

        let edge_idxs = self.outgoing.get(current).cloned().unwrap_or_default();
        let mut children = Vec::new();
        for idx in edge_idxs {
            if *budget == 0 {
                *truncated = true;
                break;
            }
            let Some(edge) = self.edges.get(idx) else {
                continue;
            };
            if !seen.insert(edge.to.clone()) {
                continue;
            }
            *budget = budget.saturating_sub(1);
            let grand = self.expand_up(&edge.to, depth.saturating_sub(1), seen, budget, truncated);
            children.push(TypeTreeNode {
                symbol: edge.to.clone(),
                kind: Some(edge.kind.clone()),
                file: Some(edge.file.clone()),
                line: Some(edge.line),
                children: grand,
            });
        }
        children
    }

    fn expand_down(
        &self,
        current: &str,
        depth: usize,
        seen: &mut HashSet<String>,
        budget: &mut usize,
        truncated: &mut bool,
    ) -> Vec<TypeTreeNode> {
        if depth == 0 || *budget == 0 {
            return Vec::new();
        }

        let edge_idxs = self.incoming.get(current).cloned().unwrap_or_default();
        let mut children = Vec::new();
        for idx in edge_idxs {
            if *budget == 0 {
                *truncated = true;
                break;
            }
            let Some(edge) = self.edges.get(idx) else {
                continue;
            };
            if !seen.insert(edge.from.clone()) {
                continue;
            }
            *budget = budget.saturating_sub(1);
            let grand =
                self.expand_down(&edge.from, depth.saturating_sub(1), seen, budget, truncated);
            children.push(TypeTreeNode {
                symbol: edge.from.clone(),
                kind: Some(edge.kind.clone()),
                file: Some(edge.file.clone()),
                line: Some(edge.line),
                children: grand,
            });
        }
        children
    }
}
