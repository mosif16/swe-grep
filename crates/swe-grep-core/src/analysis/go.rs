use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use tree_sitter::{Node, Parser};

use super::facts::{CallFact, DefinitionFact, FactBundle, ReferenceFact, RelationFact};
use super::imports::{ImportFact, ImportKind};

static CALL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap_or_else(|_| Regex::new("$^").unwrap())
});

pub(crate) fn extract(file: &str, source: &str) -> Result<FactBundle> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .is_err()
    {
        return Ok(fallback_extract(file, source));
    }

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse go source"))?;

    let mut facts = FactBundle::default();
    let root = tree.root_node();
    let mut cursor = root.walk();
    let mut caller_stack: Vec<String> = Vec::new();

    traverse(
        file,
        source,
        &mut cursor,
        &mut caller_stack,
        &mut facts.definitions,
        &mut facts.references,
        &mut facts.calls,
        &mut facts.relations,
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

        if let Some(import) = parse_import_line(trimmed) {
            facts.imports.push(ImportFact {
                file: file.to_string(),
                line: line_no,
                kind: ImportKind::Namespace,
                source: import,
                symbols: Vec::new(),
                alias: None,
            });
        }

        if let Some((name, kind)) = parse_type_line(trimmed) {
            facts.definitions.push(DefinitionFact {
                symbol: name.to_string(),
                kind: kind.to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                signature: Some(trimmed.to_string()),
            });
        }

        if let Some((qualified, signature)) = parse_func_line(trimmed) {
            current_fn = Some(qualified.clone());
            current_fn_depth = depth;
            facts.definitions.push(DefinitionFact {
                symbol: qualified,
                kind: "func".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                signature: Some(signature),
            });
        } else if let Some(caller) = current_fn.as_deref() {
            for caps in CALL_REGEX.captures_iter(line) {
                let Some(name) = caps.get(1).map(|m| m.as_str()) else {
                    continue;
                };
                if is_call_stopword(name) {
                    continue;
                }
                facts.calls.push(CallFact {
                    caller: caller.to_string(),
                    callee: name.to_string(),
                    line: line_no,
                });
            }
        }

        depth += count_braces_delta(line);
        if current_fn.is_some() && depth <= current_fn_depth {
            current_fn = None;
        }
    }

    facts
}

fn traverse(
    file: &str,
    source: &str,
    cursor: &mut tree_sitter::TreeCursor<'_>,
    caller_stack: &mut Vec<String>,
    defs: &mut Vec<DefinitionFact>,
    refs: &mut Vec<ReferenceFact>,
    calls: &mut Vec<CallFact>,
    rels: &mut Vec<RelationFact>,
    imports: &mut Vec<ImportFact>,
) {
    loop {
        let node = cursor.node();
        match node.kind() {
            "function_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    if let Some(symbol) = node_text(source, name) {
                        push_def(
                            defs,
                            symbol.clone(),
                            "func",
                            &node,
                            signature_for_decl(source, &node),
                        );
                        caller_stack.push(symbol);
                    }
                }
            }
            "method_declaration" => {
                let receiver_ty = node
                    .child_by_field_name("receiver")
                    .and_then(|recv| node_text(source, recv))
                    .and_then(|raw| parse_go_receiver_type(&raw));

                if let Some(name) = node.child_by_field_name("name") {
                    if let Some(method) = node_text(source, name) {
                        let qualified = receiver_ty
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .map(|ty| format!("{ty}::{method}"))
                            .unwrap_or_else(|| method.clone());
                        push_def(
                            defs,
                            qualified.clone(),
                            "method",
                            &node,
                            signature_for_decl(source, &node),
                        );
                        caller_stack.push(qualified);
                    }
                }
            }
            "type_spec" => {
                if let (Some(name_node), Some(type_node)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("type"),
                ) {
                    if let Some(name) = node_text(source, name_node) {
                        match type_node.kind() {
                            "struct_type" => {
                                defs.push(DefinitionFact {
                                    symbol: name.clone(),
                                    kind: "struct".to_string(),
                                    line: name_node.start_position().row as usize + 1,
                                    span: Some((
                                        name_node.start_position().row as usize + 1,
                                        name_node.end_position().row as usize + 1,
                                    )),
                                    signature: Some(struct_signature(source, &node)),
                                });

                                for (embedded, line) in struct_embeds(source, &type_node) {
                                    rels.push(RelationFact {
                                        from: name.clone(),
                                        to: embedded,
                                        kind: "embeds".to_string(),
                                        line,
                                    });
                                }
                            }
                            "interface_type" => {
                                defs.push(DefinitionFact {
                                    symbol: name.clone(),
                                    kind: "interface".to_string(),
                                    line: name_node.start_position().row as usize + 1,
                                    span: Some((
                                        name_node.start_position().row as usize + 1,
                                        name_node.end_position().row as usize + 1,
                                    )),
                                    signature: Some(interface_signature(source, &node)),
                                });

                                for (method, sig, line) in interface_methods(source, &type_node) {
                                    defs.push(DefinitionFact {
                                        symbol: format!("{name}::{method}"),
                                        kind: "interface_method".to_string(),
                                        line,
                                        span: Some((line, line)),
                                        signature: Some(sig),
                                    });
                                }

                                for (embedded, line) in interface_embeds(source, &type_node) {
                                    rels.push(RelationFact {
                                        from: name.clone(),
                                        to: embedded,
                                        kind: "embeds".to_string(),
                                        line,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            "import_spec" => {
                if let Some(fact) = import_spec_fact(file, source, &node) {
                    imports.push(fact);
                }
            }
            "call_expression" => {
                if let Some(caller) = caller_stack.last() {
                    if let Some(callee) = go_callee_text(source, &node) {
                        calls.push(CallFact {
                            caller: caller.clone(),
                            callee,
                            line: node.start_position().row as usize + 1,
                        });
                    }
                }
            }
            "identifier" | "type_identifier" => {
                if let Some(symbol) = node_text(source, node) {
                    refs.push(ReferenceFact {
                        symbol,
                        line: node.start_position().row as usize + 1,
                        context: node.parent().map(|p| p.kind().to_string()),
                    });
                }
            }
            _ => {}
        }

        if cursor.goto_first_child() {
            traverse(
                file,
                source,
                cursor,
                caller_stack,
                defs,
                refs,
                calls,
                rels,
                imports,
            );
            cursor.goto_parent();
        }

        if matches!(node.kind(), "function_declaration" | "method_declaration")
            && !caller_stack.is_empty()
        {
            caller_stack.pop();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn push_def(
    defs: &mut Vec<DefinitionFact>,
    symbol: String,
    kind: &str,
    node: &Node<'_>,
    signature: Option<String>,
) {
    let start = node.start_position().row as usize + 1;
    let end = node.end_position().row as usize + 1;
    defs.push(DefinitionFact {
        symbol,
        kind: kind.to_string(),
        line: start,
        span: Some((start, end)),
        signature,
    });
}

fn node_text(source: &str, node: Node<'_>) -> Option<String> {
    node.utf8_text(source.as_bytes())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn signature_for_decl(source: &str, node: &Node<'_>) -> Option<String> {
    let body = node.child_by_field_name("body")?;
    let start = node.start_byte();
    let end = body.start_byte();
    source
        .get(start..end)
        .map(|s| s.trim_end().to_string())
        .filter(|s| !s.is_empty())
}

fn struct_signature(source: &str, type_spec: &Node<'_>) -> String {
    node_text(source, *type_spec).unwrap_or_default()
}

fn interface_signature(source: &str, type_spec: &Node<'_>) -> String {
    node_text(source, *type_spec).unwrap_or_default()
}

fn parse_go_receiver_type(raw: &str) -> Option<String> {
    // raw: "(f *File)" or "(File)" etc.
    let inner = raw
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    if inner.is_empty() {
        return None;
    }
    // Take the last token as the type.
    let mut ty = inner.split_whitespace().last().unwrap_or("").trim();
    if ty.is_empty() {
        return None;
    }
    ty = ty.trim_start_matches('*');
    Some(ty.split('.').last().unwrap_or(ty).trim().to_string())
}

fn go_callee_text(source: &str, call_node: &Node<'_>) -> Option<String> {
    let func = call_node.child_by_field_name("function")?;
    match func.kind() {
        "identifier" | "type_identifier" => node_text(source, func),
        "selector_expression" => {
            let field = func
                .child_by_field_name("field")
                .and_then(|n| node_text(source, n))?;
            let operand = func
                .child_by_field_name("operand")
                .and_then(|n| node_text(source, n))
                .unwrap_or_default();
            if operand.is_empty() {
                Some(field)
            } else {
                // Preserve some qualification for package selectors.
                Some(format!("{operand}::{field}"))
            }
        }
        _ => node_text(source, func)
            .and_then(|s| s.split('.').last().map(|last| last.trim().to_string())),
    }
}

fn import_spec_fact(file: &str, source: &str, node: &Node<'_>) -> Option<ImportFact> {
    let path_node = node.child_by_field_name("path")?;
    let raw_path = node_text(source, path_node)?;
    let module = raw_path
        .trim_matches('"')
        .trim_matches('`')
        .trim()
        .to_string();
    if module.is_empty() {
        return None;
    }

    let alias = node
        .child_by_field_name("name")
        .and_then(|n| node_text(source, n));
    Some(ImportFact {
        file: file.to_string(),
        line: node.start_position().row as usize + 1,
        kind: ImportKind::Namespace,
        source: module,
        symbols: Vec::new(),
        alias,
    })
}

fn struct_embeds(source: &str, struct_node: &Node<'_>) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut cursor = struct_node.walk();

    fn walk(
        source: &str,
        cursor: &mut tree_sitter::TreeCursor<'_>,
        out: &mut Vec<(String, usize)>,
    ) {
        loop {
            let node = cursor.node();
            match node.kind() {
                "embedded_field" => {
                    if let Some(text) = node_text(source, node) {
                        if let Some(ty) = normalize_go_type_expr(&text) {
                            out.push((ty, node.start_position().row as usize + 1));
                        }
                    }
                }
                "field_declaration" => {
                    let has_name = node.child_by_field_name("name").is_some();
                    if !has_name {
                        if let Some(ty_node) = node.child_by_field_name("type") {
                            if let Some(text) = node_text(source, ty_node) {
                                if let Some(ty) = normalize_go_type_expr(&text) {
                                    out.push((ty, node.start_position().row as usize + 1));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }

            if cursor.goto_first_child() {
                walk(source, cursor, out);
                cursor.goto_parent();
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    walk(source, &mut cursor, &mut out);
    out
}

fn interface_methods(source: &str, iface_node: &Node<'_>) -> Vec<(String, String, usize)> {
    let mut out = Vec::new();
    let mut cursor = iface_node.walk();

    fn walk(
        source: &str,
        cursor: &mut tree_sitter::TreeCursor<'_>,
        out: &mut Vec<(String, String, usize)>,
    ) {
        loop {
            let node = cursor.node();
            if node.kind() == "method_spec" {
                if let Some(name) = node.child_by_field_name("name") {
                    if let Some(method) = node_text(source, name) {
                        let sig = node_text(source, node).unwrap_or_else(|| method.clone());
                        let line = node.start_position().row as usize + 1;
                        out.push((method, sig, line));
                    }
                }
            }

            if cursor.goto_first_child() {
                walk(source, cursor, out);
                cursor.goto_parent();
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    walk(source, &mut cursor, &mut out);
    out
}

fn interface_embeds(source: &str, iface_node: &Node<'_>) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut cursor = iface_node.walk();

    fn walk(
        source: &str,
        cursor: &mut tree_sitter::TreeCursor<'_>,
        out: &mut Vec<(String, usize)>,
    ) {
        loop {
            let node = cursor.node();
            if node.kind() == "type_identifier" {
                if let Some(parent) = node.parent() {
                    if parent.kind() == "embedded_interface" {
                        if let Some(text) = node_text(source, node) {
                            out.push((text, node.start_position().row as usize + 1));
                        }
                    }
                }
            }

            if cursor.goto_first_child() {
                walk(source, cursor, out);
                cursor.goto_parent();
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    walk(source, &mut cursor, &mut out);
    out
}

fn normalize_go_type_expr(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let trimmed = trimmed.trim_start_matches('*').trim();
    let trimmed = trimmed.split('[').next().unwrap_or(trimmed).trim();
    let trimmed = trimmed.split_whitespace().next().unwrap_or(trimmed).trim();
    let name = trimmed.split('.').last().unwrap_or(trimmed).trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn parse_import_line(trimmed: &str) -> Option<String> {
    if let Some(rest) = trimmed.strip_prefix("import ") {
        let rest = rest.trim();
        if rest.starts_with('"') || rest.starts_with('`') {
            return Some(rest.trim_matches('"').trim_matches('`').trim().to_string());
        }
    }
    None
}

fn parse_type_line(trimmed: &str) -> Option<(&str, &str)> {
    let rest = trimmed.strip_prefix("type ")?;
    let name = next_ident(rest)?;
    let after = rest.trim_start().strip_prefix(name).unwrap_or("").trim();
    if after.starts_with("struct") {
        return Some((name, "struct"));
    }
    if after.starts_with("interface") {
        return Some((name, "interface"));
    }
    None
}

fn parse_func_line(trimmed: &str) -> Option<(String, String)> {
    let rest = trimmed.strip_prefix("func ")?;
    let sig = format!("func {rest}");
    if rest.starts_with('(') {
        // method: func (r *T) M(...)
        let after_recv = rest.split(')').nth(1)?.trim_start();
        let method = next_ident(after_recv)?;
        let recv = rest.split(')').next().unwrap_or("").to_string() + ")";
        let recv_ty = parse_go_receiver_type(&recv)?;
        return Some((format!("{recv_ty}::{method}"), sig));
    }
    let name = next_ident(rest)?;
    Some((name.to_string(), sig))
}

fn next_ident(raw: &str) -> Option<&str> {
    let ident = raw
        .trim_start()
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .next()
        .unwrap_or_default();
    if ident.is_empty() { None } else { Some(ident) }
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

fn is_call_stopword(name: &str) -> bool {
    matches!(
        name,
        "if" | "for" | "switch" | "return" | "go" | "defer" | "func" | "type" | "var" | "const"
    )
}
