use std::collections::HashSet;

use anyhow::Result;
use tree_sitter::{Node, Parser};

use super::facts::{CallFact, DefinitionFact, FactBundle, ReferenceFact, RelationFact};

pub(crate) fn extract(source: &str, variant: &str) -> Result<FactBundle> {
    let mut parser = Parser::new();
    let language = match variant {
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX,
        _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
    };
    if parser.set_language(&language.into()).is_err() {
        return Ok(fallback_extract(source));
    }

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse typescript source"))?;

    let mut facts = FactBundle::default();
    let root = tree.root_node();
    let mut cursor = root.walk();
    let mut stack: Vec<String> = Vec::new();

    traverse(
        source,
        &mut cursor,
        &mut stack,
        &mut facts.definitions,
        &mut facts.references,
        &mut facts.calls,
        &mut facts.relations,
    );

    merge_relations(&mut facts.relations, fallback_relations(source));
    Ok(facts)
}

fn fallback_extract(source: &str) -> FactBundle {
    let mut facts = FactBundle::default();
    facts.relations = fallback_relations(source);
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        let line_no = idx + 1;
        if let Some(symbol) = trimmed.strip_prefix("function ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "func".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("export function ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "func".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("class ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "class".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("export class ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "class".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("interface ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "interface".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("export interface ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "interface".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("type ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "type".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("export type ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "type".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
            });
        }
    }
    facts
}

fn fallback_relations(source: &str) -> Vec<RelationFact> {
    let mut out = Vec::new();
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        let line_no = idx + 1;

        if let Some((name, bases)) = parse_class_relations(trimmed) {
            for base in bases {
                out.push(RelationFact {
                    from: name.to_string(),
                    to: base.to_string(),
                    kind: "extends".to_string(),
                    line: line_no,
                });
            }
        }
        if let Some((name, impls)) = parse_implements_relations(trimmed) {
            for iface in impls {
                out.push(RelationFact {
                    from: name.to_string(),
                    to: iface.to_string(),
                    kind: "implements".to_string(),
                    line: line_no,
                });
            }
        }
        if let Some((name, bases)) = parse_interface_extends_relations(trimmed) {
            for base in bases {
                out.push(RelationFact {
                    from: name.to_string(),
                    to: base.to_string(),
                    kind: "extends".to_string(),
                    line: line_no,
                });
            }
        }
    }
    out
}

fn merge_relations(existing: &mut Vec<RelationFact>, extra: Vec<RelationFact>) {
    if extra.is_empty() {
        return;
    }

    let mut seen: HashSet<(String, String, String, usize)> = existing
        .iter()
        .map(|rel| {
            (
                rel.from.clone(),
                rel.to.clone(),
                rel.kind.clone(),
                rel.line,
            )
        })
        .collect();

    for rel in extra {
        let key = (rel.from.clone(), rel.to.clone(), rel.kind.clone(), rel.line);
        if seen.insert(key) {
            existing.push(rel);
        }
    }
}

fn parse_class_relations(trimmed: &str) -> Option<(&str, Vec<&str>)> {
    for prefix in ["class ", "export class "].iter() {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let name = next_ident(rest)?;
            let after_name = rest.trim_start().strip_prefix(name).unwrap_or("").trim();
            let extends = after_name.split('{').next().unwrap_or("");
            if let Some(after_extends) = extends.split("extends").nth(1) {
                let base = after_extends
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim();
                if !base.is_empty() {
                    return Some((name, vec![base]));
                }
            }
        }
    }
    None
}

fn parse_interface_extends_relations(trimmed: &str) -> Option<(&str, Vec<&str>)> {
    for prefix in ["interface ", "export interface "].iter() {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let name = next_ident(rest)?;
            let after_name = rest.trim_start().strip_prefix(name).unwrap_or("").trim();
            let header = after_name.split('{').next().unwrap_or("").trim();
            let Some(after_extends) = header.split("extends").nth(1) else {
                continue;
            };
            let list = after_extends.trim();
            if list.is_empty() {
                continue;
            }
            let parts = list
                .split(',')
                .map(|p| p.trim().split_whitespace().next().unwrap_or("").trim())
                .map(|p| p.trim_end_matches('{').trim())
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>();
            if parts.is_empty() {
                continue;
            }
            return Some((name, parts));
        }
    }
    None
}

fn parse_implements_relations(trimmed: &str) -> Option<(&str, Vec<&str>)> {
    for prefix in ["class ", "export class "].iter() {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let name = next_ident(rest)?;
            let after_name = rest.trim_start().strip_prefix(name).unwrap_or("").trim();
            let header = after_name.split('{').next().unwrap_or("");
            if let Some(after_impl) = header.split("implements").nth(1) {
                let list = after_impl.trim();
                if list.is_empty() {
                    return None;
                }
                let parts = list
                    .split(',')
                    .map(|p| p.trim().trim_end_matches('{').trim())
                    .filter(|p| !p.is_empty())
                    .collect::<Vec<_>>();
                if parts.is_empty() {
                    return None;
                }
                return Some((name, parts));
            }
        }
    }
    None
}

fn next_ident(raw: &str) -> Option<&str> {
    let ident = raw
        .trim_start()
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'))
        .next()
        .unwrap_or_default();
    if ident.is_empty() {
        None
    } else {
        Some(ident)
    }
}

fn traverse(
    source: &str,
    cursor: &mut tree_sitter::TreeCursor<'_>,
    fn_stack: &mut Vec<String>,
    defs: &mut Vec<DefinitionFact>,
    refs: &mut Vec<ReferenceFact>,
    calls: &mut Vec<CallFact>,
    rels: &mut Vec<RelationFact>,
) {
    loop {
        let node = cursor.node();
        match node.kind() {
            "class_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    if let Some(symbol) = node_text(source, name) {
                        push_def(defs, symbol.clone(), node.kind(), name);
                        if let Some(superclass) = node
                            .child_by_field_name("superclass")
                            .and_then(|n| node_text(source, n))
                        {
                            rels.push(RelationFact {
                                from: symbol,
                                to: superclass,
                                kind: "extends".to_string(),
                                line: node.start_position().row as usize + 1,
                            });
                        }
                    }
                }
            }
            "interface_declaration" | "type_alias_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    if let Some(symbol) = node_text(source, name) {
                        push_def(defs, symbol, node.kind(), name);
                    }
                }
            }
            "function_declaration" | "method_definition" => {
                if let Some(name) = node.child_by_field_name("name") {
                    if let Some(symbol) = node_text(source, name) {
                        push_def(defs, symbol.clone(), "func", name);
                        fn_stack.push(symbol);
                    }
                }
            }
            "call_expression" => {
                if let Some(caller) = fn_stack.last() {
                    if let Some(callee) = node
                        .child_by_field_name("function")
                        .and_then(|n| node_text(source, n))
                    {
                        calls.push(CallFact {
                            caller: caller.clone(),
                            callee,
                            line: node.start_position().row as usize + 1,
                        });
                    }
                }
            }
            "identifier" => {
                if let Some(symbol) = node_text(source, node) {
                    refs.push(ReferenceFact {
                        symbol,
                        line: node.start_position().row as usize + 1,
                        context: node
                            .parent()
                            .map(|p| p.kind().to_string())
                            .filter(|s| !s.is_empty()),
                    });
                }
            }
            _ => {}
        }

        if cursor.goto_first_child() {
            traverse(source, cursor, fn_stack, defs, refs, calls, rels);
            cursor.goto_parent();
        }

        if matches!(node.kind(), "function_declaration" | "method_definition") && !fn_stack.is_empty()
        {
            fn_stack.pop();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn push_def(defs: &mut Vec<DefinitionFact>, symbol: String, kind: &str, name_node: Node<'_>) {
    let start = name_node.start_position().row as usize + 1;
    let end = name_node.end_position().row as usize + 1;
    defs.push(DefinitionFact {
        symbol,
        kind: kind.to_string(),
        line: start,
        span: Some((start, end)),
    });
}

fn node_text(source: &str, node: Node<'_>) -> Option<String> {
    node.utf8_text(source.as_bytes())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
