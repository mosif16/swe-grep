use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use tree_sitter::{Node, Parser};

use super::facts::{
    CallFact, DefinitionFact, FactBundle, FieldFact, MethodFact, ReferenceFact, RelationFact,
};
use super::imports::{ImportFact, ImportKind};

static CALL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap_or_else(|_| Regex::new("$^").unwrap())
});

pub(crate) fn extract(file: &str, source: &str) -> Result<FactBundle> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        return Ok(fallback_extract(file, source));
    }

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse rust source"))?;

    let mut facts = FactBundle::default();
    let lines: Vec<&str> = source.lines().collect();
    let root = tree.root_node();
    let mut cursor = root.walk();
    let mut stack: Vec<String> = Vec::new();

    traverse(
        file,
        source,
        &lines,
        &mut cursor,
        &mut stack,
        &mut facts.definitions,
        &mut facts.references,
        &mut facts.calls,
        &mut facts.relations,
        &mut facts.fields,
        &mut facts.methods,
        &mut facts.imports,
    );

    Ok(facts)
}

fn fallback_extract(file: &str, source: &str) -> FactBundle {
    let mut facts = FactBundle::default();
    let mut depth: i32 = 0;
    let mut current_fn: Option<String> = None;
    let mut current_fn_depth: i32 = 0;

    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        let line_no = idx + 1;

        if let Some(import) = parse_rust_import_line(file, trimmed, line_no) {
            facts.imports.extend(import);
        }

        if let Some((ty, tr)) = parse_impl_trait(trimmed) {
            facts.relations.push(RelationFact {
                from: ty.to_string(),
                to: tr.to_string(),
                kind: "impl_trait".to_string(),
                line: line_no,
            });
        }

        let is_fn_line = if let Some(symbol) = trimmed.strip_prefix("fn ").and_then(next_ident) {
            current_fn = Some(symbol.to_string());
            current_fn_depth = depth;
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "fn".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                visibility: None,
                doc: None,
                signature: None,
            });
            true
        } else if let Some(symbol) = trimmed.strip_prefix("pub fn ").and_then(next_ident) {
            current_fn = Some(symbol.to_string());
            current_fn_depth = depth;
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "fn".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                visibility: None,
                doc: None,
                signature: None,
            });
            true
        } else if let Some(symbol) = trimmed.strip_prefix("pub(crate) fn ").and_then(next_ident) {
            current_fn = Some(symbol.to_string());
            current_fn_depth = depth;
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "fn".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                visibility: None,
                doc: None,
                signature: None,
            });
            true
        } else {
            false
        };

        if let Some(caller) = current_fn.as_deref() {
            if !is_fn_line {
                let lowered_caller = caller.to_ascii_lowercase();
                for caps in CALL_REGEX.captures_iter(line) {
                    let Some(name) = caps.get(1).map(|m| m.as_str()) else {
                        continue;
                    };
                    if is_call_stopword(name) {
                        continue;
                    }
                    if name.to_ascii_lowercase() == lowered_caller {
                        continue;
                    }
                    facts.calls.push(CallFact {
                        caller: caller.to_string(),
                        callee: name.to_string(),
                        line: line_no,
                    });
                }
            }
        }

        if let Some(symbol) = trimmed.strip_prefix("struct ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "struct".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                visibility: None,
                doc: None,
                signature: None,
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("enum ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "enum".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                visibility: None,
                doc: None,
                signature: None,
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("trait ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "trait".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                visibility: None,
                doc: None,
                signature: None,
            });
        }

        depth += count_braces_delta(line);
        if current_fn.is_some() && depth <= current_fn_depth {
            current_fn = None;
        }
    }
    facts
}

fn parse_rust_import_line(file: &str, trimmed: &str, line_no: usize) -> Option<Vec<ImportFact>> {
    let normalized = trimmed.strip_prefix("pub ").unwrap_or(trimmed).trim_start();
    if let Some(rest) = normalized.strip_prefix("use ") {
        let stmt = rest.trim_end().trim_end_matches(';').trim();
        if stmt.is_empty() {
            return None;
        }
        return Some(parse_rust_use_stmt(file, line_no, stmt));
    }

    if let Some(rest) = normalized.strip_prefix("extern crate ") {
        let name = rest
            .trim_end()
            .trim_end_matches(';')
            .trim()
            .split_whitespace()
            .next()
            .unwrap_or("");
        if name.is_empty() {
            return None;
        }
        return Some(vec![ImportFact {
            file: file.to_string(),
            line: line_no,
            kind: ImportKind::SideEffect,
            source: name.to_string(),
            symbols: Vec::new(),
            alias: None,
        }]);
    }

    if let Some(rest) = normalized.strip_prefix("mod ") {
        let name = rest
            .trim_end()
            .trim_end_matches(';')
            .trim()
            .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .next()
            .unwrap_or("");
        if name.is_empty() || !normalized.trim_end().ends_with(';') {
            return None;
        }
        return Some(vec![ImportFact {
            file: file.to_string(),
            line: line_no,
            kind: ImportKind::SideEffect,
            source: name.to_string(),
            symbols: Vec::new(),
            alias: None,
        }]);
    }

    None
}

fn parse_rust_use_stmt(file: &str, line_no: usize, stmt: &str) -> Vec<ImportFact> {
    let stmt = stmt.trim();
    if stmt.is_empty() {
        return Vec::new();
    }

    let (prefix, items) = if let Some((before, after)) = stmt.split_once('{') {
        let prefix = before.trim().trim_end_matches("::").trim();
        let inside = after
            .rsplit_once('}')
            .map(|(inner, _)| inner)
            .unwrap_or(after);
        (prefix.to_string(), Some(inside))
    } else {
        (String::new(), None)
    };

    if let Some(list) = items {
        let mut facts = Vec::new();
        for part in list.split(',') {
            let item = part.trim();
            if item.is_empty() {
                continue;
            }
            if item == "self" {
                facts.push(ImportFact {
                    file: file.to_string(),
                    line: line_no,
                    kind: ImportKind::Named,
                    source: prefix.clone(),
                    symbols: Vec::new(),
                    alias: None,
                });
                continue;
            }
            if item == "*" {
                facts.push(ImportFact {
                    file: file.to_string(),
                    line: line_no,
                    kind: ImportKind::Namespace,
                    source: prefix.clone(),
                    symbols: Vec::new(),
                    alias: None,
                });
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
                        source: prefix.clone(),
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
                source: prefix.clone(),
                symbols: vec![item.to_string()],
                alias: None,
            });
        }
        return facts;
    }

    if stmt.ends_with("::*") {
        let src = stmt.trim_end_matches("::*").trim().to_string();
        return vec![ImportFact {
            file: file.to_string(),
            line: line_no,
            kind: ImportKind::Namespace,
            source: src,
            symbols: Vec::new(),
            alias: None,
        }];
    }

    let parts: Vec<&str> = stmt.split("::").filter(|p| !p.is_empty()).collect();
    if parts.len() <= 1 {
        return vec![ImportFact {
            file: file.to_string(),
            line: line_no,
            kind: ImportKind::Named,
            source: stmt.to_string(),
            symbols: Vec::new(),
            alias: None,
        }];
    }
    let symbol = parts[parts.len() - 1];
    let source = parts[..parts.len() - 1].join("::");
    vec![ImportFact {
        file: file.to_string(),
        line: line_no,
        kind: ImportKind::Named,
        source,
        symbols: vec![symbol.to_string()],
        alias: None,
    }]
}

fn parse_impl_trait(trimmed: &str) -> Option<(&str, &str)> {
    let rest = trimmed.strip_prefix("impl ")?;
    let header = rest.split('{').next().unwrap_or("").trim();
    let (tr, ty) = header.split_once(" for ")?;
    let tr = tr.trim();
    let ty = ty.trim();
    if tr.is_empty() || ty.is_empty() {
        None
    } else {
        Some((ty, tr))
    }
}

fn count_braces_delta(line: &str) -> i32 {
    let mut delta = 0;
    for ch in line.chars() {
        match ch {
            '{' => delta += 1,
            '}' => delta -= 1,
            _ => {}
        }
    }
    delta
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
    lines: &[&str],
    cursor: &mut tree_sitter::TreeCursor<'_>,
    fn_stack: &mut Vec<String>,
    defs: &mut Vec<DefinitionFact>,
    refs: &mut Vec<ReferenceFact>,
    calls: &mut Vec<CallFact>,
    rels: &mut Vec<RelationFact>,
    fields: &mut Vec<FieldFact>,
    methods: &mut Vec<MethodFact>,
    imports: &mut Vec<ImportFact>,
) {
    loop {
        let node = cursor.node();
        match node.kind() {
            "use_declaration" => {
                if let Some(text) = node_text(source, node) {
                    let trimmed = text.trim_start().trim_start_matches("pub ").trim_start();
                    let stmt = trimmed
                        .strip_prefix("use ")
                        .unwrap_or(trimmed)
                        .trim_end()
                        .trim_end_matches(';')
                        .trim();
                    imports.extend(parse_rust_use_stmt(
                        file,
                        node.start_position().row as usize + 1,
                        stmt,
                    ));
                }
            }
            "extern_crate_declaration" => {
                if let Some(text) = node_text(source, node) {
                    let name = text
                        .trim()
                        .trim_start_matches("extern crate ")
                        .trim_end_matches(';')
                        .trim()
                        .split_whitespace()
                        .next()
                        .unwrap_or("");
                    if !name.is_empty() {
                        imports.push(ImportFact {
                            file: file.to_string(),
                            line: node.start_position().row as usize + 1,
                            kind: ImportKind::SideEffect,
                            source: name.to_string(),
                            symbols: Vec::new(),
                            alias: None,
                        });
                    }
                }
            }
            "mod_item" => {
                if let Some(name) = node.child_by_field_name("name") {
                    if let Some(symbol) = node_text(source, name) {
                        push_definition_fact(lines, source, defs, symbol, "mod", node, name);
                    }
                }
                if let Some(text) = node_text(source, node) {
                    let trimmed = text.trim();
                    if trimmed.ends_with(';') && !trimmed.contains('{') {
                        let name = trimmed
                            .trim_start_matches("mod ")
                            .trim_end_matches(';')
                            .trim()
                            .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                            .next()
                            .unwrap_or("");
                        if !name.is_empty() {
                            imports.push(ImportFact {
                                file: file.to_string(),
                                line: node.start_position().row as usize + 1,
                                kind: ImportKind::SideEffect,
                                source: name.to_string(),
                                symbols: Vec::new(),
                                alias: None,
                            });
                        }
                    }
                }
            }
            "struct_item" | "enum_item" | "trait_item" | "const_item" | "static_item"
            | "type_item" => {
                if let Some(name) = node.child_by_field_name("name") {
                    if let Some(symbol) = node_text(source, name) {
                        let kind = match node.kind() {
                            "struct_item" => "struct",
                            "enum_item" => "enum",
                            "trait_item" => "trait",
                            "const_item" => "const",
                            "static_item" => "static",
                            "type_item" => "type",
                            other => other,
                        };
                        push_definition_fact(lines, source, defs, symbol.clone(), kind, node, name);
                        if node.kind() == "struct_item" {
                            extract_struct_fields(lines, source, node, &symbol, fields);
                        }
                    }
                }
            }
            "impl_item" => {
                let ty = node
                    .child_by_field_name("type")
                    .and_then(|n| node_text(source, n));
                let tr = node
                    .child_by_field_name("trait")
                    .and_then(|n| node_text(source, n));
                if let (Some(from), Some(to)) = (ty, tr) {
                    rels.push(RelationFact {
                        from,
                        to,
                        kind: "impl_trait".to_string(),
                        line: node.start_position().row as usize + 1,
                    });
                }
            }
            "function_item" => {
                if let Some(name) = node.child_by_field_name("name") {
                    if let Some(symbol) = node_text(source, name) {
                        push_definition_fact(lines, source, defs, symbol.clone(), "fn", node, name);
                        if let Some(owner) = find_impl_owner_type(source, &node) {
                            methods.push(MethodFact {
                                owner,
                                name: symbol.clone(),
                                line: node.start_position().row as usize + 1,
                                span: Some((
                                    node.start_position().row as usize + 1,
                                    node.end_position().row as usize + 1,
                                )),
                                visibility: extract_rust_visibility(source, node),
                                doc: extract_rust_doc_before(
                                    lines,
                                    node.start_position().row as usize,
                                ),
                                signature: extract_signature_lines(
                                    lines,
                                    node.start_position().row as usize,
                                ),
                            });
                        }
                        fn_stack.push(qualify_rust_function(source, &node, &symbol));
                    }
                }
            }
            "call_expression" | "method_call_expression" => {
                if let Some(caller) = fn_stack.last() {
                    if let Some(callee) = callee_text(source, &node) {
                        calls.push(CallFact {
                            caller: caller.clone(),
                            callee,
                            line: node.start_position().row as usize + 1,
                        });
                    }
                }
            }
            "macro_invocation" => {
                if let Some(caller) = fn_stack.last() {
                    extract_macro_calls(source, &node, caller, calls);
                }
            }
            "identifier" | "type_identifier" => {
                if !is_definition_identifier(&node) {
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
            }
            _ => {}
        }

        if cursor.goto_first_child() {
            traverse(
                file, source, lines, cursor, fn_stack, defs, refs, calls, rels, fields, methods,
                imports,
            );
            cursor.goto_parent();
        }

        if node.kind() == "function_item" && !fn_stack.is_empty() {
            fn_stack.pop();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn node_text(source: &str, node: Node<'_>) -> Option<String> {
    node.utf8_text(source.as_bytes())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn is_definition_identifier(node: &Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    matches!(
        parent.kind(),
        "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "impl_item"
            | "const_item"
            | "static_item"
            | "type_item"
            | "mod_item"
    ) && parent
        .child_by_field_name("name")
        .map(|name| name.id() == node.id())
        .unwrap_or(false)
}

fn qualify_rust_function(source: &str, function_node: &Node<'_>, name: &str) -> String {
    let mut current = function_node.parent();
    while let Some(node) = current {
        if node.kind() == "impl_item" {
            if let Some(ty) = node.child_by_field_name("type") {
                if let Some(ty_text) = node_text(source, ty) {
                    return format!("{ty_text}::{name}");
                }
            }
        }
        current = node.parent();
    }
    name.to_string()
}

fn callee_text(source: &str, call_node: &Node<'_>) -> Option<String> {
    if call_node.kind() == "call_expression" {
        if let Some(func) = call_node.child_by_field_name("function") {
            // For scoped calls like Type::method(), extract the full qualified path
            return extract_qualified_callee(source, func);
        }
    }
    if call_node.kind() == "method_call_expression" {
        // For method calls like obj.method(), try to infer the receiver type
        // and create a qualified name when possible
        if let Some(name) = call_node.child_by_field_name("name") {
            let method_name = node_text(source, name)?;
            // Try to get receiver type for better qualification
            if let Some(receiver) = call_node.child_by_field_name("value") {
                if let Some(receiver_type) = infer_receiver_type(source, receiver) {
                    return Some(format!("{}::{}", receiver_type, method_name));
                }
            }
            return Some(method_name);
        }
    }
    node_text(source, *call_node)
}

/// Extract a qualified callee name, preserving Type::method format
fn extract_qualified_callee(source: &str, func_node: Node<'_>) -> Option<String> {
    match func_node.kind() {
        "scoped_identifier" => {
            // Handle Type::method or module::Type::method
            node_text(source, func_node)
        }
        "field_expression" => {
            // Handle self.method or object.method
            if let Some(field) = func_node.child_by_field_name("field") {
                let method = node_text(source, field)?;
                if let Some(value) = func_node.child_by_field_name("value") {
                    if let Some(receiver_type) = infer_receiver_type(source, value) {
                        return Some(format!("{}::{}", receiver_type, method));
                    }
                }
                return Some(method);
            }
            node_text(source, func_node)
        }
        "identifier" => {
            // Simple function call like foo()
            node_text(source, func_node)
        }
        _ => node_text(source, func_node),
    }
}

/// Try to infer the type of a receiver expression for method calls
fn infer_receiver_type(source: &str, receiver: Node<'_>) -> Option<String> {
    match receiver.kind() {
        "self" => Some("Self".to_string()),
        "identifier" => {
            // Could look up variable types from context, but for now skip
            None
        }
        "call_expression" => {
            // For chained calls like Foo::new().bar(), try to get Foo
            if let Some(func) = receiver.child_by_field_name("function") {
                if func.kind() == "scoped_identifier" {
                    // Extract the type part from Type::method
                    let text = node_text(source, func)?;
                    return text.split("::").next().map(|s| s.to_string());
                }
            }
            None
        }
        _ => None,
    }
}

fn extract_macro_calls(source: &str, node: &Node<'_>, caller: &str, calls: &mut Vec<CallFact>) {
    let Ok(text) = node.utf8_text(source.as_bytes()) else {
        return;
    };
    let base_line = node.start_position().row as usize + 1;

    for caps in CALL_REGEX.captures_iter(text) {
        let Some(name) = caps.get(1).map(|m| m.as_str()) else {
            continue;
        };
        if is_call_stopword(name) {
            continue;
        }
        calls.push(CallFact {
            caller: caller.to_string(),
            callee: name.to_string(),
            line: base_line,
        });
    }
}

fn is_call_stopword(name: &str) -> bool {
    matches!(
        name,
        "if" | "for"
            | "while"
            | "loop"
            | "match"
            | "switch"
            | "return"
            | "await"
            | "self"
            | "super"
            | "this"
            | "macro_rules"
            | "let"
            | "var"
            | "fn"
            | "func"
            | "function"
            | "class"
            | "struct"
            | "enum"
            | "impl"
            | "where"
    )
}

fn push_definition_fact(
    lines: &[&str],
    source: &str,
    defs: &mut Vec<DefinitionFact>,
    symbol: String,
    kind: &str,
    item_node: Node<'_>,
    _name_node: Node<'_>,
) {
    let start = item_node.start_position().row as usize + 1;
    let end = item_node.end_position().row as usize + 1;
    defs.push(DefinitionFact {
        symbol,
        kind: kind.to_string(),
        line: start,
        span: Some((start, end)),
        visibility: extract_rust_visibility(source, item_node),
        doc: extract_rust_doc_before(lines, item_node.start_position().row as usize),
        signature: extract_signature_lines(lines, item_node.start_position().row as usize),
    });
}

fn extract_rust_visibility(source: &str, node: Node<'_>) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            return node_text(source, child);
        }
    }
    None
}

fn extract_signature_lines(lines: &[&str], start_row: usize) -> Option<String> {
    if lines.is_empty() || start_row >= lines.len() {
        return None;
    }
    let end_row = usize::min(lines.len().saturating_sub(1), start_row.saturating_add(6));
    let mut buf = String::new();
    for row in start_row..=end_row {
        let line = lines.get(row).copied().unwrap_or_default();
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(line.trim());
        if buf.contains('{') || buf.contains(';') {
            break;
        }
    }
    let compact = buf.split_whitespace().collect::<Vec<_>>().join(" ");
    let cutoff = compact
        .find('{')
        .map(|idx| idx + 1)
        .or_else(|| compact.find(';').map(|idx| idx + 1))
        .unwrap_or(compact.len());
    let out = compact[..cutoff].trim().to_string();
    if out.is_empty() { None } else { Some(out) }
}

fn extract_rust_doc_before(lines: &[&str], start_row: usize) -> Option<String> {
    if start_row == 0 || lines.is_empty() {
        return None;
    }

    let mut idx = start_row.saturating_sub(1);
    let mut collected = Vec::new();
    let mut budget = 40usize;

    while budget > 0 {
        let line = lines.get(idx).copied().unwrap_or_default();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }

        if trimmed.starts_with("///") || trimmed.starts_with("//!") {
            collected.push(trimmed.to_string());
            budget -= 1;
            if idx == 0 {
                break;
            }
            idx -= 1;
            continue;
        }

        if trimmed.ends_with("*/") {
            let mut block = vec![trimmed.to_string()];
            budget -= 1;
            while budget > 0 && idx > 0 {
                idx -= 1;
                let prev = lines
                    .get(idx)
                    .copied()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if prev.is_empty() {
                    break;
                }
                block.push(prev.clone());
                budget -= 1;
                if prev.contains("/**") || prev.contains("/*!") {
                    break;
                }
            }
            block.reverse();
            collected.extend(block);
            break;
        }

        break;
    }

    clean_doc_lines(&collected)
}

fn clean_doc_lines(raw: &[String]) -> Option<String> {
    if raw.is_empty() {
        return None;
    }

    let mut out = Vec::new();
    for line in raw {
        let trimmed = line.trim();
        if trimmed.starts_with("///") {
            out.push(trimmed.trim_start_matches("///").trim_start().to_string());
            continue;
        }
        if trimmed.starts_with("//!") {
            out.push(trimmed.trim_start_matches("//!").trim_start().to_string());
            continue;
        }
        if trimmed.starts_with("/**") || trimmed.starts_with("/*!") {
            let cleaned = trimmed
                .trim_start_matches("/**")
                .trim_start_matches("/*!")
                .trim_end_matches("*/")
                .trim();
            if !cleaned.is_empty() {
                out.push(cleaned.to_string());
            }
            continue;
        }
        if trimmed.ends_with("*/") {
            let cleaned = trimmed.trim_end_matches("*/").trim();
            if !cleaned.is_empty() {
                out.push(cleaned.to_string());
            }
            continue;
        }
        let cleaned = trimmed
            .trim_start_matches('*')
            .trim_start()
            .trim_end_matches("*/")
            .trim();
        if !cleaned.is_empty() {
            out.push(cleaned.to_string());
        }
    }

    let joined = out.join("\n").trim().to_string();
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn extract_struct_fields(
    lines: &[&str],
    source: &str,
    struct_node: Node<'_>,
    owner: &str,
    fields: &mut Vec<FieldFact>,
) {
    let body = struct_node
        .child_by_field_name("body")
        .or_else(|| find_first_child_of_kind(struct_node, "field_declaration_list"));
    let Some(list) = body else {
        return;
    };

    let mut cursor = list.walk();
    for child in list.named_children(&mut cursor) {
        if child.kind() != "field_declaration" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let Some(name) = node_text(source, name_node) else {
            continue;
        };
        let type_repr = child
            .child_by_field_name("type")
            .and_then(|n| node_text(source, n));
        fields.push(FieldFact {
            owner: owner.to_string(),
            name,
            line: child.start_position().row as usize + 1,
            span: Some((
                child.start_position().row as usize + 1,
                child.end_position().row as usize + 1,
            )),
            visibility: extract_rust_visibility(source, child),
            doc: extract_rust_doc_before(lines, child.start_position().row as usize),
            type_repr,
        });
    }
}

fn find_first_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

fn find_impl_owner_type(source: &str, function_node: &Node<'_>) -> Option<String> {
    let mut current = function_node.parent();
    while let Some(node) = current {
        if node.kind() == "impl_item" {
            return node
                .child_by_field_name("type")
                .and_then(|ty| node_text(source, ty));
        }
        current = node.parent();
    }
    None
}
