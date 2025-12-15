use std::collections::HashSet;

use anyhow::Result;
use tree_sitter::{Node, Parser};

use super::facts::{CallFact, DefinitionFact, FactBundle, ReferenceFact, RelationFact};
use super::imports::{ImportFact, ImportKind};

pub(crate) fn extract(file: &str, source: &str) -> Result<FactBundle> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_swift::LANGUAGE.into())
        .is_err()
    {
        return Ok(fallback_extract(file, source));
    }

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse swift source"))?;

    let mut facts = FactBundle::default();
    let root = tree.root_node();
    let mut cursor = root.walk();
    let mut stack: Vec<String> = Vec::new();

    traverse(
        file,
        source,
        &mut cursor,
        &mut stack,
        &mut facts.definitions,
        &mut facts.references,
        &mut facts.calls,
        &mut facts.relations,
        &mut facts.imports,
    );

    merge_relations(&mut facts.relations, fallback_relations(source));
    Ok(facts)
}

fn fallback_extract(file: &str, source: &str) -> FactBundle {
    let mut facts = FactBundle::default();
    facts.relations = fallback_relations(source);
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        let line_no = idx + 1;
        if let Some(module) = parse_swift_import_line(trimmed) {
            facts.imports.push(ImportFact {
                file: file.to_string(),
                line: line_no,
                kind: ImportKind::SideEffect,
                source: module,
                symbols: Vec::new(),
                alias: None,
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("func ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "func".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                signature: None,
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("struct ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "struct".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                signature: None,
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("class ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "class".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                signature: None,
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("protocol ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "protocol".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                signature: None,
            });
        }
    }
    facts
}

fn parse_swift_import_line(trimmed: &str) -> Option<String> {
    let trimmed = trimmed.trim_start_matches("@testable ").trim_start();
    let rest = trimmed.strip_prefix("import ")?;
    let first = rest
        .trim()
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .next()
        .unwrap_or("")
        .trim();
    if first.is_empty() {
        None
    } else {
        Some(first.to_string())
    }
}

fn fallback_relations(source: &str) -> Vec<RelationFact> {
    let mut out = Vec::new();
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        let line_no = idx + 1;

        if let Some((name, bases)) = parse_type_decl(trimmed) {
            for base in bases {
                out.push(RelationFact {
                    from: name.to_string(),
                    to: base.to_string(),
                    kind: "conforms_to".to_string(),
                    line: line_no,
                });
            }
        }
        if let Some(ext) = trimmed.strip_prefix("extension ").and_then(next_ident) {
            out.push(RelationFact {
                from: ext.to_string(),
                to: ext.to_string(),
                kind: "extension_of".to_string(),
                line: line_no,
            });
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
        .map(|rel| (rel.from.clone(), rel.to.clone(), rel.kind.clone(), rel.line))
        .collect();

    for rel in extra {
        let key = (rel.from.clone(), rel.to.clone(), rel.kind.clone(), rel.line);
        if seen.insert(key) {
            existing.push(rel);
        }
    }
}

fn parse_type_decl(trimmed: &str) -> Option<(&str, Vec<&str>)> {
    for prefix in ["struct ", "class ", "actor ", "enum "].iter() {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let name = next_ident(rest)?;
            let after_name = rest.trim_start().strip_prefix(name).unwrap_or("");
            let after_colon = after_name.split('{').next().unwrap_or("").trim();
            let after_colon = after_colon.strip_prefix(':')?.trim();
            if after_colon.is_empty() {
                return None;
            }
            let bases = after_colon
                .split(',')
                .map(|part| part.trim())
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>();
            return Some((name, bases));
        }
    }
    None
}

fn next_ident(raw: &str) -> Option<&str> {
    let ident = raw
        .trim_start()
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .next()
        .unwrap_or_default();
    if ident.is_empty() { None } else { Some(ident) }
}

fn traverse(
    file: &str,
    source: &str,
    cursor: &mut tree_sitter::TreeCursor<'_>,
    fn_stack: &mut Vec<String>,
    defs: &mut Vec<DefinitionFact>,
    refs: &mut Vec<ReferenceFact>,
    calls: &mut Vec<CallFact>,
    rels: &mut Vec<RelationFact>,
    imports: &mut Vec<ImportFact>,
) {
    loop {
        let node = cursor.node();
        match node.kind() {
            "import_declaration" => {
                let line = node.start_position().row as usize + 1;
                if let Some(text) = node_text(source, node) {
                    let after = text.split("import").nth(1).unwrap_or("").trim_start();
                    let module = after
                        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                        .next()
                        .unwrap_or("")
                        .trim();
                    if !module.is_empty() {
                        imports.push(ImportFact {
                            file: file.to_string(),
                            line,
                            kind: ImportKind::SideEffect,
                            source: module.to_string(),
                            symbols: Vec::new(),
                            alias: None,
                        });
                    }
                }
            }
            "class_declaration" | "struct_declaration" | "protocol_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    if let Some(symbol) = node_text(source, name) {
                        push_def(defs, symbol, node.kind(), name);
                    }
                }
            }
            "function_declaration" => {
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
            "identifier" | "simple_identifier" => {
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
            traverse(
                file, source, cursor, fn_stack, defs, refs, calls, rels, imports,
            );
            cursor.goto_parent();
        }

        if node.kind() == "function_declaration" && !fn_stack.is_empty() {
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
        signature: None,
    });
}

fn node_text(source: &str, node: Node<'_>) -> Option<String> {
    node.utf8_text(source.as_bytes())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
