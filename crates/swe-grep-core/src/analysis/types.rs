use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::facts::FileFacts;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TypeGraphIndex {
    outgoing: HashMap<String, Vec<TypeRelationEdge>>,
    incoming: HashMap<String, Vec<TypeRelationEdge>>,
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

        infer_go_edges(files, &mut edges);
        infer_python_protocol_edges(files, &mut edges);

        let mut outgoing: HashMap<String, Vec<usize>> = HashMap::new();
        let mut incoming: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, edge) in edges.iter().enumerate() {
            outgoing.entry(edge.from.clone()).or_default().push(idx);
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
            TypeDirection::Up => {
                self.expand_up(&root, depth, &mut seen, &mut budget, &mut truncated)
            }
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

fn infer_go_edges(files: &[FileFacts], edges: &mut Vec<TypeRelationEdge>) {
    let mut struct_methods: HashMap<String, HashSet<String>> = HashMap::new();
    let mut iface_methods: HashMap<String, HashSet<String>> = HashMap::new();
    let mut iface_embeds: HashMap<String, Vec<String>> = HashMap::new();
    let mut type_locations: HashMap<String, (String, usize)> = HashMap::new();

    for file in files.iter().filter(|f| f.language == "go") {
        for def in &file.facts.definitions {
            if def.kind == "struct" || def.kind == "interface" {
                type_locations
                    .entry(def.symbol.clone())
                    .or_insert((file.file.clone(), def.line));
            }
            if def.kind == "method" {
                if let Some((ty, method)) = def.symbol.split_once("::") {
                    if !ty.is_empty() && !method.is_empty() {
                        struct_methods
                            .entry(ty.to_string())
                            .or_default()
                            .insert(method.to_string());
                    }
                }
            }
            if def.kind == "interface_method" {
                if let Some((iface, method)) = def.symbol.split_once("::") {
                    if !iface.is_empty() && !method.is_empty() {
                        iface_methods
                            .entry(iface.to_string())
                            .or_default()
                            .insert(method.to_string());
                    }
                }
            }
        }

        for rel in &file.facts.relations {
            if rel.kind == "embeds" {
                iface_embeds
                    .entry(rel.from.clone())
                    .or_default()
                    .push(rel.to.clone());
            }
        }
    }

    if struct_methods.is_empty() || iface_methods.is_empty() {
        return;
    }

    let mut memo: HashMap<String, HashSet<String>> = HashMap::new();
    let expanded: HashMap<String, HashSet<String>> = iface_methods
        .keys()
        .map(|iface| {
            let methods =
                expand_go_interface_methods(iface, &iface_methods, &iface_embeds, &mut memo);
            (iface.clone(), methods)
        })
        .collect();

    let mut existing: HashSet<(String, String, String)> = edges
        .iter()
        .map(|e| (e.from.clone(), e.to.clone(), e.kind.clone()))
        .collect();

    for (ty, methods) in &struct_methods {
        for (iface, required) in &expanded {
            if required.is_empty() {
                continue;
            }
            if required.is_subset(methods) {
                let kind = "implements".to_string();
                let key = (ty.clone(), iface.clone(), kind.clone());
                if !existing.insert(key) {
                    continue;
                }
                let (file, line) = type_locations
                    .get(ty)
                    .cloned()
                    .unwrap_or_else(|| ("".to_string(), 1));
                edges.push(TypeRelationEdge {
                    from: ty.clone(),
                    to: iface.clone(),
                    kind,
                    file,
                    line,
                });
            }
        }
    }
}

fn expand_go_interface_methods(
    iface: &str,
    direct: &HashMap<String, HashSet<String>>,
    embeds: &HashMap<String, Vec<String>>,
    memo: &mut HashMap<String, HashSet<String>>,
) -> HashSet<String> {
    if let Some(existing) = memo.get(iface) {
        return existing.clone();
    }

    let mut out = direct.get(iface).cloned().unwrap_or_default();
    if let Some(children) = embeds.get(iface) {
        for child in children {
            if child == iface {
                continue;
            }
            let nested = expand_go_interface_methods(child, direct, embeds, memo);
            out.extend(nested);
        }
    }

    memo.insert(iface.to_string(), out.clone());
    out
}

fn infer_python_protocol_edges(files: &[FileFacts], edges: &mut Vec<TypeRelationEdge>) {
    let mut class_methods: HashMap<String, HashSet<String>> = HashMap::new();
    let mut class_locations: HashMap<String, (String, usize)> = HashMap::new();
    let mut protocol_classes: HashSet<String> = HashSet::new();

    for file in files.iter().filter(|f| f.language == "python") {
        for def in &file.facts.definitions {
            if def.kind == "class" {
                class_locations
                    .entry(def.symbol.clone())
                    .or_insert((file.file.clone(), def.line));
            }
            if def.kind == "method" || def.kind == "async_method" {
                if let Some((cls, method)) = def.symbol.split_once("::") {
                    if !cls.is_empty() && !method.is_empty() {
                        class_methods
                            .entry(cls.to_string())
                            .or_default()
                            .insert(method.to_string());
                    }
                }
            }
        }
        for rel in &file.facts.relations {
            if rel.kind == "inherits" && rel.to == "Protocol" {
                protocol_classes.insert(rel.from.clone());
            }
        }
    }

    if protocol_classes.is_empty() {
        return;
    }

    let mut protocol_methods: HashMap<String, HashSet<String>> = HashMap::new();
    for proto in &protocol_classes {
        if let Some(methods) = class_methods.get(proto) {
            protocol_methods.insert(proto.clone(), methods.clone());
        }
    }

    let mut existing: HashSet<(String, String, String)> = edges
        .iter()
        .map(|e| (e.from.clone(), e.to.clone(), e.kind.clone()))
        .collect();

    for (cls, methods) in &class_methods {
        if protocol_classes.contains(cls) {
            continue;
        }
        for (proto, required) in &protocol_methods {
            if required.is_empty() {
                continue;
            }
            if required.is_subset(methods) {
                let kind = "implements_protocol".to_string();
                let key = (cls.clone(), proto.clone(), kind.clone());
                if !existing.insert(key) {
                    continue;
                }
                let (file, line) = class_locations
                    .get(cls)
                    .cloned()
                    .unwrap_or_else(|| ("".to_string(), 1));
                edges.push(TypeRelationEdge {
                    from: cls.clone(),
                    to: proto.clone(),
                    kind,
                    file,
                    line,
                });
            }
        }
    }
}

impl TypeGraphIndex {
    pub fn add_edges(&mut self, edges: impl IntoIterator<Item = TypeRelationEdge>) {
        for edge in edges {
            self.add_edge(edge);
        }
    }

    pub fn remove_edges(&mut self, edges: impl IntoIterator<Item = TypeRelationEdge>) {
        for edge in edges {
            self.remove_edge(&edge);
        }
    }

    fn add_edge(&mut self, edge: TypeRelationEdge) {
        self.outgoing
            .entry(edge.from.clone())
            .or_default()
            .push(edge.clone());
        self.incoming.entry(edge.to.clone()).or_default().push(edge);
    }

    fn remove_edge(&mut self, edge: &TypeRelationEdge) {
        remove_index_edge(&mut self.outgoing, &edge.from, edge);
        remove_index_edge(&mut self.incoming, &edge.to, edge);
    }
}

fn remove_index_edge(
    map: &mut HashMap<String, Vec<TypeRelationEdge>>,
    key: &str,
    edge: &TypeRelationEdge,
) {
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
