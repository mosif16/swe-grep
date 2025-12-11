use std::collections::HashSet;
use std::path::Path;

#[derive(Clone, Copy)]
struct QueryTemplate {
    template: &'static str,
    needs_type_like: bool,
    needs_component: bool,
    needs_hook: bool,
}

impl QueryTemplate {
    const fn base(template: &'static str) -> Self {
        Self {
            template,
            needs_type_like: false,
            needs_component: false,
            needs_hook: false,
        }
    }

    const fn type_like(template: &'static str) -> Self {
        Self {
            template,
            needs_type_like: true,
            needs_component: false,
            needs_hook: false,
        }
    }

    const fn component(template: &'static str) -> Self {
        Self {
            template,
            needs_type_like: false,
            needs_component: true,
            needs_hook: false,
        }
    }

    const fn hook(template: &'static str) -> Self {
        Self {
            template,
            needs_type_like: false,
            needs_component: false,
            needs_hook: true,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct SymbolShape {
    is_type_like: bool,
    is_component: bool,
    is_hook: bool,
}

impl SymbolShape {
    fn from_symbol(symbol: &str) -> Self {
        let is_component = symbol
            .chars()
            .next()
            .map(|ch| ch.is_uppercase())
            .unwrap_or(false);

        Self {
            is_type_like: is_component,
            is_component,
            is_hook: symbol.starts_with("use") && symbol.len() > 3,
        }
    }
}

const TYPESCRIPT_QUERIES: &[QueryTemplate] = &[
    QueryTemplate::base("{symbol}<"),
    QueryTemplate::base("{symbol} <"),
    QueryTemplate::base("<{symbol}"),
    QueryTemplate::base("</{symbol}"),
    QueryTemplate::base("{symbol} extends"),
    QueryTemplate::base("type {symbol}"),
    QueryTemplate::base("interface {symbol}"),
    QueryTemplate::base("const {symbol}"),
    QueryTemplate::base("export const {symbol}"),
    QueryTemplate::base("function {symbol}"),
    QueryTemplate::base("export function {symbol}"),
    QueryTemplate::base("{symbol}("),
    QueryTemplate::base("{symbol} satisfies"),
    QueryTemplate::base("namespace {symbol}"),
    QueryTemplate::base("export default {symbol}"),
    QueryTemplate::base("{symbol} props"),
    QueryTemplate::base("{symbol}:"),
    QueryTemplate::hook("{symbol}("),
    QueryTemplate::hook("{symbol}<{{"),
    QueryTemplate::type_like("<{symbol} "),
    QueryTemplate::type_like("<{symbol} />"),
    QueryTemplate::type_like("{symbol}Props"),
    QueryTemplate::type_like("{symbol}Component"),
    QueryTemplate::component("<{symbol} {{..."),
    QueryTemplate::component("React.memo({symbol}"),
    QueryTemplate::component("React.forwardRef({symbol}"),
];

const SWIFT_QUERIES: &[QueryTemplate] = &[
    QueryTemplate::base("func {symbol}"),
    QueryTemplate::base("func {symbol}("),
    QueryTemplate::base("func {symbol}<"),
    QueryTemplate::base("{symbol} async"),
    QueryTemplate::base("@MainActor func {symbol}"),
    QueryTemplate::base("{symbol}("),
    QueryTemplate::base(".{symbol}"),
    QueryTemplate::base("self.{symbol}"),
    QueryTemplate::base("await {symbol}"),
    QueryTemplate::type_like("@{symbol}"),
    QueryTemplate::type_like(": {symbol}"),
    QueryTemplate::type_like("extension {symbol}"),
    QueryTemplate::type_like("where {symbol}"),
];

const RUST_QUERIES: &[QueryTemplate] = &[
    QueryTemplate::base("fn {symbol}"),
    QueryTemplate::base("impl {symbol}"),
    QueryTemplate::base("trait {symbol}"),
    QueryTemplate::base("pub(crate) {symbol}"),
    QueryTemplate::base("{symbol}::<"),
    QueryTemplate::base("::{symbol}"),
    QueryTemplate::base("macro_rules! {symbol}"),
];

const TYPESCRIPT_ATTRIBUTE_QUERIES: &[&str] = &[
    "@{symbol}",
    "@{symbol}(",
    "@{symbol} ",
];

const SWIFT_ATTRIBUTE_QUERIES: &[&str] = &[
    "@{symbol}",
    "@{symbol}(",
    "@{symbol} ",
];

const TYPESCRIPT_AST_PATTERNS: &[&str] = &[
    "(identifier) @id (#eq? @id \"{needle}\")",
    "(call_expression function: (identifier) @id (#eq? @id \"{needle}\"))",
    "(call_expression function: (member_expression property: (property_identifier) @id (#eq? @id \"{needle}\")))",
    "(class_declaration name: (identifier) @id (#eq? @id \"{needle}\"))",
    "(interface_declaration name: (identifier) @id (#eq? @id \"{needle}\"))",
    "(interface_declaration name: (type_identifier) @id (#eq? @id \"{needle}\"))",
    "(type_alias_declaration name: (identifier) @id (#eq? @id \"{needle}\"))",
    "(type_alias_declaration name: (type_identifier) @id (#eq? @id \"{needle}\"))",
    "(method_definition name: (property_identifier) @id (#eq? @id \"{needle}\"))",
    "(lexical_declaration (variable_declarator name: (identifier) @id (#eq? @id \"{needle}\"))))",
    "(jsx_opening_element name: (identifier) @id (#eq? @id \"{needle}\"))",
    "(lexical_declaration (variable_declarator name: (identifier) @id (#eq? @id \"{needle}\") value: (arrow_function)))",
    "(export_statement value: (identifier) @id (#eq? @id \"{needle}\"))",
    "(export_statement (export_clause (export_specifier name: (identifier) @id (#eq? @id \"{needle}\"))))",
    "(jsx_attribute name: (property_identifier) @id (#eq? @id \"{needle}\"))",
    "(binary_expression left: (identifier) @id (#eq? @id \"{needle}\") operator: \"satisfies\")",
];

const SWIFT_AST_PATTERNS: &[&str] = &[
    "(function_declaration name: (identifier) @id (#eq? @id \"{needle}\"))",
    "(protocol_declaration name: (identifier) @id (#eq? @id \"{needle}\"))",
    "(protocol_member_declaration (function_declaration name: (identifier) @id) (#eq? @id \"{needle}\"))",
    "(initializer_declaration name: (identifier) @id (#eq? @id \"{needle}\"))",
    "(class_declaration body: (member_declaration_list (member_declaration (function_declaration name: (identifier) @id (#eq? @id \"{needle}\")))))",
    "(struct_declaration body: (member_declaration_list (member_declaration (function_declaration name: (identifier) @id (#eq? @id \"{needle}\")))))",
    "(extension_declaration body: (member_declaration_list (member_declaration (function_declaration name: (identifier) @id (#eq? @id \"{needle}\")))))",
    "(actor_declaration body: (member_declaration_list (member_declaration (function_declaration name: (identifier) @id (#eq? @id \"{needle}\")))))",
    "(member_access_expression name: (identifier) @id (#eq? @id \"{needle}\"))",
    "(function_call_expression function: (identifier) @id (#eq? @id \"{needle}\"))",
    "(await_expression (function_call_expression function: (identifier) @id (#eq? @id \"{needle}\")))",
    "(attribute attribute_name: (identifier) @id (#eq? @id \"{needle}\"))",
    "(extension_declaration protocol_conformance: (identifier) @id (#eq? @id \"{needle}\"))",
    "(generic_argument_clause (identifier) @id (#eq? @id \"{needle}\"))",
];

const RUST_AST_PATTERNS: &[&str] = &[
    "(function_item name: (identifier) @id (#eq? @id \"{needle}\"))",
    "(impl_item type: (type_path (path_segment name: (identifier) @id (#eq? @id \"{needle}\"))))",
    "(trait_item name: (identifier) @id (#eq? @id \"{needle}\"))",
    "(struct_item name: (identifier) @id (#eq? @id \"{needle}\"))",
    "(enum_item name: (identifier) @id (#eq? @id \"{needle}\"))",
    "(macro_invocation macro: (identifier) @id (#eq? @id \"{needle}\"))",
    "(impl_item trait: (trait_ref path: (scoped_identifier path: (identifier) @id (#eq? @id \"{needle}\"))))",
    "(impl_item trait: (trait_ref path: (type_identifier) @id (#eq? @id \"{needle}\")))",
];

const DEFAULT_AST_PATTERNS: &[&str] = &["(identifier) @id (#eq? @id \"{needle}\")"];

pub fn query_patterns_for(language: &str, symbol: &str) -> Vec<String> {
    let normalized = language.to_ascii_lowercase();
    let shape = SymbolShape::from_symbol(symbol);
    let templates = match normalized.as_str() {
        "typescript" | "ts" | "tsx" => TYPESCRIPT_QUERIES,
        "swift" => SWIFT_QUERIES,
        "rust" => RUST_QUERIES,
        _ => &[],
    };

    render_query_templates(templates, symbol, shape)
}

pub fn attribute_patterns_for(language: &str, symbol: &str) -> Vec<String> {
    let trimmed = symbol.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let needle = trimmed.trim_start_matches('@').trim();
    if needle.is_empty() {
        return Vec::new();
    }

    let normalized = language.to_ascii_lowercase();
    let templates: &[&str] = match normalized.as_str() {
        "typescript" | "ts" | "tsx" | "javascript" | "js" | "jsx" => TYPESCRIPT_ATTRIBUTE_QUERIES,
        "swift" => SWIFT_ATTRIBUTE_QUERIES,
        _ => &[],
    };

    templates
        .iter()
        .map(|template| template.replace("{symbol}", needle))
        .collect()
}

fn render_query_templates(
    templates: &[QueryTemplate],
    symbol: &str,
    shape: SymbolShape,
) -> Vec<String> {
    let mut rendered = Vec::new();
    for template in templates {
        if template.needs_type_like && !shape.is_type_like {
            continue;
        }
        if template.needs_component && !shape.is_component {
            continue;
        }
        if template.needs_hook && !shape.is_hook {
            continue;
        }
        rendered.push(template.template.replace("{symbol}", symbol));
    }
    rendered
}

pub fn ast_patterns_for(language: &str, symbol: &str) -> Vec<String> {
    let needle = symbol.trim();
    if needle.is_empty() {
        return vec!["(identifier) @id".to_string()];
    }

    let normalized = language.to_ascii_lowercase();
    let templates = match normalized.as_str() {
        "typescript" | "ts" | "tsx" => TYPESCRIPT_AST_PATTERNS,
        "swift" => SWIFT_AST_PATTERNS,
        "rust" => RUST_AST_PATTERNS,
        _ => DEFAULT_AST_PATTERNS,
    };

    templates
        .iter()
        .map(|template| template.replace("{needle}", needle))
        .collect()
}

pub fn expand_language_hint(language: Option<&str>) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let Some(raw) = language else {
        return tokens;
    };
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return tokens;
    }

    if normalized.starts_with("auto-") {
        let remainder = normalized.trim_start_matches("auto-");
        let parts: Vec<&str> = remainder
            .split(|ch| matches!(ch, '-' | '+' | '|' | ','))
            .filter(|part| !part.is_empty())
            .collect();
        for part in parts {
            tokens.extend(expand_language_token(part));
        }
    } else {
        let parts: Vec<&str> = normalized
            .split(|ch| matches!(ch, '+' | '|' | ','))
            .filter(|part| !part.is_empty())
            .collect();
        if parts.is_empty() {
            tokens.extend(expand_language_token(&normalized));
        } else {
            for part in parts {
                tokens.extend(expand_language_token(part));
            }
        }
    }

    let mut dedup: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for token in tokens {
        if seen.insert(token.clone()) {
            dedup.push(token);
        }
    }
    dedup
}

pub fn expand_language_token(token: &str) -> Vec<String> {
    match token {
        "typescript" | "ts" => vec!["ts".to_string(), "tsx".to_string()],
        "tsx" => vec!["tsx".to_string()],
        "swift" => vec!["swift".to_string()],
        "rust" | "rs" => vec!["rust".to_string()],
        "javascript" | "js" => vec!["js".to_string(), "jsx".to_string()],
        "jsx" => vec!["jsx".to_string()],
        "kotlin" | "kt" => vec!["kt".to_string(), "kts".to_string()],
        "kts" => vec!["kts".to_string()],
        "python" | "py" => vec!["py".to_string()],
        "css" => vec!["css".to_string()],
        "scss" | "sass" => vec!["scss".to_string()],
        "swiftui" => vec!["swift".to_string()],
        other => vec![other.to_string()],
    }
}

pub fn languages_include(tokens: &[String], needle: &str) -> bool {
    tokens.iter().any(|token| token == needle)
}

pub fn code_extensions_for_languages(languages: &[String]) -> Vec<String> {
    let mut results: Vec<String> = Vec::new();
    let mut push_unique = |ext: &str| {
        if !results
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(ext))
        {
            results.push(ext.to_string());
        }
    };

    if languages.is_empty() {
        for ext in [
            "rs", "swift", "ts", "tsx", "js", "jsx", "kt", "kts", "py", "java", "go", "rb", "php",
            "cs", "cpp", "cxx", "cc", "c", "h", "hpp", "hh", "m", "mm", "scala", "dart", "css",
            "scss", "sass", "sql",
        ] {
            push_unique(ext);
        }
        return results;
    }

    for lang in languages {
        match lang.as_str() {
            "swift" => push_unique("swift"),
            "tsx" => push_unique("tsx"),
            "ts" | "typescript" => {
                push_unique("ts");
                push_unique("tsx");
            }
            "rust" => push_unique("rs"),
            "js" | "javascript" => {
                push_unique("js");
                push_unique("jsx");
            }
            "jsx" => push_unique("jsx"),
            "kt" | "kts" | "kotlin" => {
                push_unique("kt");
                push_unique("kts");
            }
            "py" | "python" => push_unique("py"),
            "css" => push_unique("css"),
            "scss" | "sass" => push_unique("scss"),
            other => push_unique(other),
        }
    }

    results
}

pub fn extensions_for_languages(
    languages: &[String],
    include_strings: bool,
) -> Option<Vec<&'static str>> {
    let mut results: Vec<&'static str> = Vec::new();
    if languages.is_empty() {
        return None;
    }
    for lang in languages {
        match lang.as_str() {
            "swift" => {
                if !results.contains(&"swift") {
                    results.push("swift");
                }
            }
            "tsx" => {
                if !results.contains(&"tsx") {
                    results.push("tsx");
                }
            }
            "ts" | "typescript" => {
                if !results.contains(&"ts") {
                    results.push("ts");
                }
                if !results.contains(&"tsx") {
                    results.push("tsx");
                }
            }
            "rust" => {
                if !results.contains(&"rs") {
                    results.push("rs");
                }
            }
            "js" | "javascript" => {
                if !results.contains(&"js") {
                    results.push("js");
                }
                if !results.contains(&"jsx") {
                    results.push("jsx");
                }
            }
            "jsx" => {
                if !results.contains(&"jsx") {
                    results.push("jsx");
                }
            }
            "kt" | "kts" | "kotlin" => {
                if !results.contains(&"kt") {
                    results.push("kt");
                }
                if !results.contains(&"kts") {
                    results.push("kts");
                }
            }
            "py" | "python" => {
                if !results.contains(&"py") {
                    results.push("py");
                }
            }
            "css" => {
                if !results.contains(&"css") {
                    results.push("css");
                }
            }
            "scss" | "sass" => {
                if !results.contains(&"scss") {
                    results.push("scss");
                }
            }
            _ => {}
        }
    }
    if include_strings {
        for ext in ["xcstrings", "strings", "arb", "xliff", "json"] {
            if !results.contains(&ext) {
                results.push(ext);
            }
        }
    }
    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}

pub fn detect_language_from_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    match ext.to_ascii_lowercase().as_str() {
        "rs" => Some("rust"),
        "swift" => Some("swift"),
        "ts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "js" => Some("javascript"),
        "jsx" => Some("jsx"),
        "py" => Some("python"),
        "kt" | "kts" => Some("kotlin"),
        "css" => Some("css"),
        "scss" => Some("scss"),
        _ => None,
    }
}
