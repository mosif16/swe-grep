use std::collections::HashSet;

use anyhow::Result;
use tree_sitter::{Node, Parser};

use super::facts::{CallFact, DefinitionFact, FactBundle, ReferenceFact, RelationFact};
use super::imports::{ExportFact, ExportKind, ImportFact, ImportKind};

pub(crate) fn extract(file: &str, source: &str, variant: &str) -> Result<FactBundle> {
    let mut parser = Parser::new();
    let language = match variant {
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX,
        _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
    };
    if parser.set_language(&language.into()).is_err() {
        return Ok(fallback_extract(file, source));
    }

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse typescript source"))?;

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
        &mut facts.exports,
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
        if let Some(imports) = parse_ts_import_like_line(file, trimmed, line_no) {
            facts.imports.extend(imports);
        }
        if let Some(exports) = parse_ts_export_like_line(file, trimmed, line_no) {
            facts.exports.extend(exports);
        }
        if trimmed.trim_start().starts_with("export ") && trimmed.contains(" from ") {
            facts
                .imports
                .extend(parse_ts_reexport_statement_text(file, line_no, trimmed));
        }
        if let Some(symbol) = trimmed.strip_prefix("function ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "func".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                signature: None,
            });
        }
        if let Some(symbol) = trimmed
            .strip_prefix("export function ")
            .and_then(next_ident)
        {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "func".to_string(),
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
        if let Some(symbol) = trimmed.strip_prefix("export class ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "class".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                signature: None,
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("interface ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "interface".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                signature: None,
            });
        }
        if let Some(symbol) = trimmed
            .strip_prefix("export interface ")
            .and_then(next_ident)
        {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "interface".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                signature: None,
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("type ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "type".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                signature: None,
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("export type ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "type".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                signature: None,
            });
        }
    }
    facts
}

fn parse_ts_import_like_line(file: &str, trimmed: &str, line_no: usize) -> Option<Vec<ImportFact>> {
    let trimmed = trimmed.trim();
    if trimmed.starts_with("import ") {
        return Some(parse_ts_import_statement_text(file, line_no, trimmed));
    }
    if let Some(idx) = trimmed.find("require(") {
        let after = &trimmed[idx + "require(".len()..];
        let lit = after.split(')').next().unwrap_or("").trim();
        let source = unquote_js_string_literal(lit)?;
        return Some(vec![ImportFact {
            file: file.to_string(),
            line: line_no,
            kind: ImportKind::SideEffect,
            source,
            symbols: Vec::new(),
            alias: None,
        }]);
    }
    None
}

fn parse_ts_export_like_line(file: &str, trimmed: &str, line_no: usize) -> Option<Vec<ExportFact>> {
    let trimmed = trimmed.trim();
    if !trimmed.starts_with("export ") {
        return None;
    }
    Some(parse_ts_export_statement_text(file, line_no, trimmed))
}

fn parse_ts_import_statement_text(file: &str, line_no: usize, text: &str) -> Vec<ImportFact> {
    let text = text.trim_end().trim_end_matches(';').trim();
    let after_import = text.strip_prefix("import ").unwrap_or(text).trim_start();
    if after_import.starts_with('"') || after_import.starts_with('\'') {
        let Some(source) = unquote_js_string_literal(after_import) else {
            return Vec::new();
        };
        return vec![ImportFact {
            file: file.to_string(),
            line: line_no,
            kind: ImportKind::SideEffect,
            source,
            symbols: Vec::new(),
            alias: None,
        }];
    }

    let Some((clause, from_part)) = split_on_from(after_import) else {
        return Vec::new();
    };
    let Some(source) = extract_last_string_literal(from_part) else {
        return Vec::new();
    };

    let mut facts = Vec::new();
    let clause = clause.trim();
    if clause.is_empty() {
        return facts;
    }

    if let Some((default_part, rest)) = clause.split_once(',') {
        let default_name = default_part.trim();
        if !default_name.is_empty() {
            facts.push(ImportFact {
                file: file.to_string(),
                line: line_no,
                kind: ImportKind::Default,
                source: source.clone(),
                symbols: vec![default_name.to_string()],
                alias: None,
            });
        }
        facts.extend(parse_ts_import_clause(file, line_no, rest.trim(), &source));
        return facts;
    }

    facts.extend(parse_ts_import_clause(file, line_no, clause, &source));
    facts
}

fn parse_ts_import_clause(
    file: &str,
    line_no: usize,
    clause: &str,
    source: &str,
) -> Vec<ImportFact> {
    let clause = clause.trim();
    if clause.is_empty() {
        return Vec::new();
    }

    if let Some(star_pos) = clause.find('*') {
        let after = &clause[star_pos..];
        if let Some((_, alias)) = after.split_once("as") {
            let name = alias.trim();
            if !name.is_empty() {
                return vec![ImportFact {
                    file: file.to_string(),
                    line: line_no,
                    kind: ImportKind::Namespace,
                    source: source.to_string(),
                    symbols: Vec::new(),
                    alias: Some(name.to_string()),
                }];
            }
        }
    }

    if clause.contains('{') {
        let inner = clause.split_once('}').map(|(a, _)| a).unwrap_or(clause);
        let inner = inner
            .split_once('{')
            .map(|(_, b)| b)
            .unwrap_or(inner)
            .trim();
        let mut facts = Vec::new();
        for part in inner.split(',') {
            let item = part.trim();
            if item.is_empty() {
                continue;
            }
            if let Some((name, alias)) = item.split_once(" as ") {
                let name = name.trim();
                let alias = alias.trim();
                if !name.is_empty() && !alias.is_empty() {
                    facts.push(ImportFact {
                        file: file.to_string(),
                        line: line_no,
                        kind: ImportKind::Named,
                        source: source.to_string(),
                        symbols: vec![name.to_string()],
                        alias: Some(alias.to_string()),
                    });
                }
                continue;
            }
            facts.push(ImportFact {
                file: file.to_string(),
                line: line_no,
                kind: ImportKind::Named,
                source: source.to_string(),
                symbols: vec![item.to_string()],
                alias: None,
            });
        }
        return facts;
    }

    vec![ImportFact {
        file: file.to_string(),
        line: line_no,
        kind: ImportKind::Default,
        source: source.to_string(),
        symbols: vec![clause.to_string()],
        alias: None,
    }]
}

fn parse_ts_export_statement_text(file: &str, line_no: usize, text: &str) -> Vec<ExportFact> {
    let text = text.trim_end().trim_end_matches(';').trim();
    let normalized = text.strip_prefix("export ").unwrap_or(text).trim_start();

    if normalized.starts_with("default ") {
        let rest = normalized
            .strip_prefix("default ")
            .unwrap_or("")
            .trim_start();
        if let Some(name) = rest.strip_prefix("function ").and_then(next_ident) {
            return vec![ExportFact {
                file: file.to_string(),
                line: line_no,
                symbol: name.to_string(),
                kind: ExportKind::Function,
                is_default: true,
            }];
        }
        if let Some(name) = rest.strip_prefix("class ").and_then(next_ident) {
            return vec![ExportFact {
                file: file.to_string(),
                line: line_no,
                symbol: name.to_string(),
                kind: ExportKind::Class,
                is_default: true,
            }];
        }
        let ident = rest
            .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'))
            .next()
            .unwrap_or("")
            .trim();
        return vec![ExportFact {
            file: file.to_string(),
            line: line_no,
            symbol: if ident.is_empty() {
                "default".to_string()
            } else {
                ident.to_string()
            },
            kind: ExportKind::Const,
            is_default: true,
        }];
    }

    for (prefix, kind) in [
        ("function ", ExportKind::Function),
        ("class ", ExportKind::Class),
        ("interface ", ExportKind::Interface),
        ("type ", ExportKind::Type),
        ("const ", ExportKind::Const),
        ("enum ", ExportKind::Enum),
    ] {
        if let Some(name) = normalized.strip_prefix(prefix).and_then(next_ident) {
            return vec![ExportFact {
                file: file.to_string(),
                line: line_no,
                symbol: name.to_string(),
                kind,
                is_default: false,
            }];
        }
    }

    if normalized.starts_with('{') {
        let inner = normalized
            .split_once('}')
            .map(|(a, _)| a)
            .unwrap_or(normalized);
        let inner = inner.trim_start_matches('{').trim();
        let mut exports = Vec::new();
        for part in inner.split(',') {
            let item = part.trim();
            if item.is_empty() {
                continue;
            }
            let (name, _) = item.split_once(" as ").unwrap_or((item, ""));
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            exports.push(ExportFact {
                file: file.to_string(),
                line: line_no,
                symbol: name.to_string(),
                kind: ExportKind::Const,
                is_default: false,
            });
        }
        return exports;
    }

    Vec::new()
}

fn split_on_from(raw: &str) -> Option<(&str, &str)> {
    let mut parts = raw.rsplitn(2, " from ");
    let from_part = parts.next()?;
    let clause = parts.next()?;
    Some((clause, from_part))
}

fn extract_last_string_literal(raw: &str) -> Option<String> {
    let s = raw.trim();
    if let Some(start) = s.rfind('"') {
        if let Some(end) = s[..start].rfind('"') {
            let lit = &s[end..=start];
            return unquote_js_string_literal(lit);
        }
    }
    if let Some(start) = s.rfind('\'') {
        if let Some(end) = s[..start].rfind('\'') {
            let lit = &s[end..=start];
            return unquote_js_string_literal(lit);
        }
    }
    unquote_js_string_literal(s)
}

fn unquote_js_string_literal(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.len() < 2 {
        return None;
    }
    let bytes = raw.as_bytes();
    let quote = bytes[0];
    if (quote != b'"' && quote != b'\'') || bytes[raw.len() - 1] != quote {
        return None;
    }
    Some(raw[1..raw.len() - 1].to_string())
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
        .map(|rel| (rel.from.clone(), rel.to.clone(), rel.kind.clone(), rel.line))
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
                let base = after_extends.split_whitespace().next().unwrap_or("").trim();
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
    exports: &mut Vec<ExportFact>,
) {
    loop {
        let node = cursor.node();
        match node.kind() {
            "import_statement" => {
                let line = node.start_position().row as usize + 1;
                if let Some(text) = node_text(source, node) {
                    imports.extend(parse_ts_import_statement_text(file, line, &text));
                }
            }
            "export_statement" => {
                let line = node.start_position().row as usize + 1;
                if let Some(text) = node_text(source, node) {
                    imports.extend(parse_ts_reexport_statement_text(file, line, &text));
                    exports.extend(parse_ts_export_statement_text(file, line, &text));
                }
            }
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

                if is_require_call(source, &node) {
                    let line = node.start_position().row as usize + 1;
                    if let Some(text) = node_text(source, node) {
                        if let Some(source_mod) = extract_require_source(&text) {
                            imports.push(ImportFact {
                                file: file.to_string(),
                                line,
                                kind: ImportKind::SideEffect,
                                source: source_mod,
                                symbols: Vec::new(),
                                alias: None,
                            });
                        }
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
            traverse(
                file, source, cursor, fn_stack, defs, refs, calls, rels, imports, exports,
            );
            cursor.goto_parent();
        }

        if matches!(node.kind(), "function_declaration" | "method_definition")
            && !fn_stack.is_empty()
        {
            fn_stack.pop();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn parse_ts_reexport_statement_text(file: &str, line_no: usize, text: &str) -> Vec<ImportFact> {
    let text = text.trim_end().trim_end_matches(';').trim();
    let normalized = text.strip_prefix("export ").unwrap_or(text).trim_start();
    if !normalized.contains(" from ") {
        return Vec::new();
    }

    if normalized.starts_with('*') {
        let Some((_, from_part)) = split_on_from(normalized) else {
            return Vec::new();
        };
        let Some(source) = extract_last_string_literal(from_part) else {
            return Vec::new();
        };
        return vec![ImportFact {
            file: file.to_string(),
            line: line_no,
            kind: ImportKind::Reexport,
            source,
            symbols: Vec::new(),
            alias: None,
        }];
    }

    if !normalized.starts_with('{') {
        return Vec::new();
    }

    let Some((spec, from_part)) = split_on_from(normalized) else {
        return Vec::new();
    };
    let Some(source) = extract_last_string_literal(from_part) else {
        return Vec::new();
    };

    let inner = spec.split_once('}').map(|(a, _)| a).unwrap_or(spec);
    let inner = inner.trim_start_matches('{').trim();
    let mut facts = Vec::new();
    for part in inner.split(',') {
        let item = part.trim();
        if item.is_empty() {
            continue;
        }
        if let Some((name, alias)) = item.split_once(" as ") {
            let name = name.trim();
            let alias = alias.trim();
            if !name.is_empty() && !alias.is_empty() {
                facts.push(ImportFact {
                    file: file.to_string(),
                    line: line_no,
                    kind: ImportKind::Reexport,
                    source: source.clone(),
                    symbols: vec![name.to_string()],
                    alias: Some(alias.to_string()),
                });
            }
            continue;
        }
        facts.push(ImportFact {
            file: file.to_string(),
            line: line_no,
            kind: ImportKind::Reexport,
            source: source.clone(),
            symbols: vec![item.to_string()],
            alias: None,
        });
    }
    facts
}

fn is_require_call(source: &str, node: &Node<'_>) -> bool {
    let Some(function) = node.child_by_field_name("function") else {
        return false;
    };
    node_text(source, function)
        .as_deref()
        .map(|t| t == "require")
        .unwrap_or(false)
}

fn extract_require_source(call_text: &str) -> Option<String> {
    let Some(idx) = call_text.find("require(") else {
        return None;
    };
    let after = &call_text[idx + "require(".len()..];
    let lit = after.split(')').next().unwrap_or("").trim();
    unquote_js_string_literal(lit)
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
