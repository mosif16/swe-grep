use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use tree_sitter::{Node, Parser};

use super::facts::{CallFact, DefinitionFact, FactBundle, ReferenceFact, RelationFact};

static CALL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap_or_else(|_| Regex::new("$^").unwrap())
});

pub(crate) fn extract(source: &str) -> Result<FactBundle> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        return Ok(fallback_extract(source));
    }

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse rust source"))?;

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

    Ok(facts)
}

fn fallback_extract(source: &str) -> FactBundle {
    let mut facts = FactBundle::default();
    let mut depth: i32 = 0;
    let mut current_fn: Option<String> = None;
    let mut current_fn_depth: i32 = 0;

    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        let line_no = idx + 1;

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
            });
            true
        } else if let Some(symbol) = trimmed
            .strip_prefix("pub(crate) fn ")
            .and_then(next_ident)
        {
            current_fn = Some(symbol.to_string());
            current_fn_depth = depth;
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "fn".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
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
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("enum ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "enum".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
            });
        }
        if let Some(symbol) = trimmed.strip_prefix("trait ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "trait".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
            });
        }

        depth += count_braces_delta(line);
        if current_fn.is_some() && depth <= current_fn_depth {
            current_fn = None;
        }
    }
    facts
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
            "struct_item" | "enum_item" | "trait_item" => {
                if let Some(name) = node.child_by_field_name("name") {
                    if let Some(symbol) = node_text(source, name) {
                        push_def(defs, symbol, node.kind(), name);
                    }
                }
            }
            "impl_item" => {
                let ty = node.child_by_field_name("type").and_then(|n| node_text(source, n));
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
                        push_def(defs, symbol.clone(), "fn", name);
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
            traverse(source, cursor, fn_stack, defs, refs, calls, rels);
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

fn is_definition_identifier(node: &Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    matches!(
        parent.kind(),
        "function_item" | "struct_item" | "enum_item" | "trait_item" | "impl_item"
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
        _ => node_text(source, func_node)
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
        _ => None
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
        "if"
            | "for"
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
