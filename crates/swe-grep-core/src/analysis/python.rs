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
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .is_err()
    {
        return Ok(fallback_extract(file, source));
    }

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("failed to parse python source"))?;

    let mut facts = FactBundle::default();
    let root = tree.root_node();
    let mut cursor = root.walk();

    let mut class_stack: Vec<String> = Vec::new();
    let mut caller_stack: Vec<String> = Vec::new();

    traverse(
        file,
        source,
        &mut cursor,
        &mut class_stack,
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

        if let Some((imports, kind)) = parse_import_facts(file, trimmed, line_no) {
            for import in imports {
                facts.imports.push(import);
            }
            // Keep `kind` to avoid unused variable in future extensions.
            let _ = kind;
        }

        if let Some((name, signature, is_async)) = parse_def_line(trimmed) {
            let kind = if is_async { "async_func" } else { "func" };
            current_fn = Some(name.clone());
            current_fn_depth = depth;
            facts.definitions.push(DefinitionFact {
                symbol: name,
                kind: kind.to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                visibility: None,
                doc: None,
                signature: Some(signature),
            });
        } else if let Some(symbol) = trimmed.strip_prefix("class ").and_then(next_ident) {
            facts.definitions.push(DefinitionFact {
                symbol: symbol.to_string(),
                kind: "class".to_string(),
                line: line_no,
                span: Some((line_no, line_no)),
                visibility: None,
                doc: None,
                signature: Some(trimmed.to_string()),
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

        depth += count_block_delta(line);
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
    class_stack: &mut Vec<String>,
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
            "import_statement" | "import_from_statement" => {
                let line = node.start_position().row as usize + 1;
                for fact in import_facts_from_node(file, source, &node, line) {
                    imports.push(fact);
                }
            }
            "decorator" => {
                if let Some(name) = decorator_name(source, &node) {
                    defs.push(DefinitionFact {
                        symbol: name,
                        kind: "decorator".to_string(),
                        line: node.start_position().row as usize + 1,
                        span: Some((
                            node.start_position().row as usize + 1,
                            node.end_position().row as usize + 1,
                        )),
                        visibility: None,
                        doc: None,
                        signature: node_text(source, node),
                    });
                }
            }
            "class_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    if let Some(name) = node_text(source, name_node) {
                        defs.push(DefinitionFact {
                            symbol: name.clone(),
                            kind: "class".to_string(),
                            line: name_node.start_position().row as usize + 1,
                            span: Some((
                                name_node.start_position().row as usize + 1,
                                name_node.end_position().row as usize + 1,
                            )),
                            visibility: None,
                            doc: None,
                            signature: signature_for_block_header(source, &node),
                        });

                        for (base, line) in class_bases(source, &node) {
                            rels.push(RelationFact {
                                from: name.clone(),
                                to: base,
                                kind: "inherits".to_string(),
                                line,
                            });
                        }

                        class_stack.push(name);
                    }
                }
            }
            "function_definition" | "async_function_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    if let Some(name) = node_text(source, name_node) {
                        let is_method = !class_stack.is_empty();
                        let container = class_stack.last().cloned();
                        let qualified = match container.as_deref() {
                            Some(class) if is_method => format!("{class}::{name}"),
                            _ => name.clone(),
                        };
                        let signature = signature_for_block_header(source, &node);
                        let is_async = node.kind() == "async_function_definition"
                            || signature
                                .as_deref()
                                .unwrap_or_default()
                                .trim_start()
                                .starts_with("async def");
                        let kind = if is_async {
                            if is_method {
                                "async_method"
                            } else {
                                "async_func"
                            }
                        } else if is_method {
                            "method"
                        } else {
                            "func"
                        };

                        defs.push(DefinitionFact {
                            symbol: qualified.clone(),
                            kind: kind.to_string(),
                            line: name_node.start_position().row as usize + 1,
                            span: Some((
                                name_node.start_position().row as usize + 1,
                                name_node.end_position().row as usize + 1,
                            )),
                            visibility: None,
                            doc: None,
                            signature,
                        });

                        caller_stack.push(qualified);
                    }
                }
            }
            "call" => {
                if let Some(caller) = caller_stack.last() {
                    if let Some(callee) = python_callee_text(source, &node) {
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
                class_stack,
                caller_stack,
                defs,
                refs,
                calls,
                rels,
                imports,
            );
            cursor.goto_parent();
        }

        if node.kind() == "class_definition" && !class_stack.is_empty() {
            class_stack.pop();
        }
        if matches!(
            node.kind(),
            "function_definition" | "async_function_definition"
        ) && !caller_stack.is_empty()
        {
            caller_stack.pop();
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

fn signature_for_block_header(source: &str, node: &Node<'_>) -> Option<String> {
    let body = node.child_by_field_name("body")?;
    source
        .get(node.start_byte()..body.start_byte())
        .map(|s| s.trim_end().to_string())
        .filter(|s| !s.is_empty())
}

fn decorator_name(source: &str, node: &Node<'_>) -> Option<String> {
    let raw = node_text(source, *node)?;
    let trimmed = raw.trim_start_matches('@').trim();
    let base = trimmed.split('(').next().unwrap_or(trimmed).trim();
    let name = base.split('.').last().unwrap_or(base).trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn class_bases(source: &str, class_node: &Node<'_>) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let Some(bases) = class_node.child_by_field_name("superclasses") else {
        return out;
    };
    let line = bases.start_position().row as usize + 1;
    let Ok(text) = bases.utf8_text(source.as_bytes()) else {
        return out;
    };

    let inner = text
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    for part in inner.split(',') {
        let base = normalize_py_base(part.trim());
        if !base.is_empty() {
            out.push((base, line));
        }
    }
    out
}

fn normalize_py_base(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let without_generics = trimmed.split('[').next().unwrap_or(trimmed).trim();
    let without_args = without_generics
        .split('(')
        .next()
        .unwrap_or(without_generics)
        .trim();
    let name = without_args
        .split('.')
        .last()
        .unwrap_or(without_args)
        .trim();
    name.to_string()
}

fn python_callee_text(source: &str, call_node: &Node<'_>) -> Option<String> {
    let func = call_node.child_by_field_name("function")?;
    match func.kind() {
        "identifier" => node_text(source, func),
        "attribute" => {
            let attr = func
                .child_by_field_name("attribute")
                .and_then(|n| node_text(source, n))?;
            let obj = func
                .child_by_field_name("object")
                .and_then(|n| node_text(source, n));
            match obj.as_deref() {
                Some(obj) if !obj.is_empty() => Some(format!("{obj}::{attr}")),
                _ => Some(attr),
            }
        }
        _ => node_text(source, func)
            .and_then(|s| s.split('.').last().map(|last| last.trim().to_string())),
    }
}

fn import_facts_from_node(
    file: &str,
    source: &str,
    node: &Node<'_>,
    line: usize,
) -> Vec<ImportFact> {
    let mut out = Vec::new();
    let Some(text) = node_text(source, *node) else {
        return out;
    };

    if node.kind() == "import_statement" {
        let rest = text.trim_start_matches("import").trim();
        for part in rest.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (module, alias) = parse_import_part(part);
            if module.is_empty() {
                continue;
            }
            out.push(ImportFact {
                file: file.to_string(),
                line,
                kind: ImportKind::Namespace,
                source: module,
                symbols: Vec::new(),
                alias,
            });
        }
        return out;
    }

    if node.kind() == "import_from_statement" {
        // from X import a as b, c
        let Some((from_part, import_part)) = text.split_once("import") else {
            return out;
        };
        let module = from_part
            .trim_start_matches("from")
            .trim()
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if module.is_empty() {
            return out;
        }
        let imported = import_part.trim();
        for part in imported.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if part == "*" {
                out.push(ImportFact {
                    file: file.to_string(),
                    line,
                    kind: ImportKind::Named,
                    source: module.clone(),
                    symbols: vec!["*".to_string()],
                    alias: None,
                });
                continue;
            }
            let (sym, alias) = parse_import_part(part);
            if sym.is_empty() {
                continue;
            }
            out.push(ImportFact {
                file: file.to_string(),
                line,
                kind: ImportKind::Named,
                source: module.clone(),
                symbols: vec![sym],
                alias,
            });
        }
    }

    out
}

fn parse_import_part(part: &str) -> (String, Option<String>) {
    let part = part.trim();
    if part.is_empty() {
        return (String::new(), None);
    }
    if let Some((left, right)) = part.split_once(" as ") {
        let left = left.trim();
        let right = right.trim();
        let name = left.split_whitespace().next().unwrap_or("").trim();
        let alias = right.split_whitespace().next().unwrap_or("").trim();
        (
            name.to_string(),
            if alias.is_empty() {
                None
            } else {
                Some(alias.to_string())
            },
        )
    } else {
        let name = part.split_whitespace().next().unwrap_or("").trim();
        (name.to_string(), None)
    }
}

fn parse_import_facts(
    file: &str,
    trimmed: &str,
    line: usize,
) -> Option<(Vec<ImportFact>, &'static str)> {
    if let Some(rest) = trimmed.strip_prefix("import ") {
        let (module, alias) = parse_import_part(rest);
        if module.is_empty() {
            return None;
        }
        return Some((
            vec![ImportFact {
                file: file.to_string(),
                line,
                kind: ImportKind::Namespace,
                source: module,
                symbols: Vec::new(),
                alias,
            }],
            "import",
        ));
    }
    if let Some(rest) = trimmed.strip_prefix("from ") {
        let Some((module_part, imports_part)) = rest.split_once(" import ") else {
            return None;
        };
        let module = module_part.split_whitespace().next().unwrap_or("").trim();
        if module.is_empty() {
            return None;
        }
        let mut facts = Vec::new();
        for part in imports_part.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if part == "*" {
                facts.push(ImportFact {
                    file: file.to_string(),
                    line,
                    kind: ImportKind::Named,
                    source: module.to_string(),
                    symbols: vec!["*".to_string()],
                    alias: None,
                });
                continue;
            }
            let (sym, alias) = parse_import_part(part);
            if sym.is_empty() {
                continue;
            }
            facts.push(ImportFact {
                file: file.to_string(),
                line,
                kind: ImportKind::Named,
                source: module.to_string(),
                symbols: vec![sym],
                alias,
            });
        }
        if facts.is_empty() {
            return None;
        }
        return Some((facts, "from_import"));
    }
    None
}

fn parse_def_line(trimmed: &str) -> Option<(String, String, bool)> {
    if let Some(rest) = trimmed.strip_prefix("def ") {
        let name = next_ident(rest)?.to_string();
        return Some((name, trimmed.to_string(), false));
    }
    if let Some(rest) = trimmed.strip_prefix("async def ") {
        let name = next_ident(rest)?.to_string();
        return Some((name, trimmed.to_string(), true));
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

fn count_block_delta(line: &str) -> i32 {
    // Very rough heuristic: increase when line ends with ':' and decrease on dedent isn't tracked.
    // We only use this to limit fallback call attribution; treat blank lines as neutral.
    if line.trim().ends_with(':') { 1 } else { 0 }
}

fn is_call_stopword(name: &str) -> bool {
    matches!(
        name,
        "if" | "for" | "while" | "return" | "await" | "async" | "def" | "class" | "lambda"
    )
}
