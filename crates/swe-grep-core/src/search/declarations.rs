//! Snapshot-local declaration records. Body spans are deliberately never indexed.
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use regex::RegexSet;

#[derive(Default)]
pub(crate) struct DeclarationIndex {
    names: HashMap<usize, Vec<String>>,
    patterns: RefCell<Option<(Vec<String>, Option<RegexSet>)>>,
}

impl DeclarationIndex {
    pub(crate) fn new(path: &Path, source: &str) -> Self {
        let mut index = Self::default();
        let language = crate::language::detect_language_from_path(path);
        if let Some(grammar) = grammar(language) {
            let mut parser = tree_sitter::Parser::new();
            if parser.set_language(&grammar).is_ok() {
                if let Some(tree) = parser.parse(source, None) {
                    let mut cursor = tree.walk();
                    loop {
                        index.visit(cursor.node(), source, language);
                        if cursor.goto_first_child() {
                            continue;
                        }
                        loop {
                            if cursor.goto_next_sibling() {
                                break;
                            }
                            if !cursor.goto_parent() {
                                return index;
                            }
                        }
                    }
                }
            }
        }
        // Unsupported grammars get conservative lexical declarations only.
        // Supported languages never depend on quote/comment guessing for name rows.
        let tokens = tokenize(source);
        for pair in tokens.windows(2) {
            if matches!(
                pair[0].text,
                "fn" | "def"
                    | "func"
                    | "function"
                    | "class"
                    | "struct"
                    | "enum"
                    | "interface"
                    | "trait"
                    | "protocol"
                    | "namespace"
            ) && pair[1].identifier()
            {
                index.add(pair[1].line, pair[1].text);
            }
        }
        index
    }

    fn visit(&mut self, node: tree_sitter::Node<'_>, source: &str, language: Option<&str>) {
        let declaration = match language {
            Some("rust") => matches!(
                node.kind(),
                "function_item"
                    | "function_signature_item"
                    | "struct_item"
                    | "enum_item"
                    | "trait_item"
                    | "union_item"
                    | "type_item"
                    | "const_item"
                    | "static_item"
                    | "mod_item"
                    | "macro_definition"
                    | "field_declaration"
                    | "enum_variant"
            ),
            Some("python") => matches!(node.kind(), "function_definition" | "class_definition"),
            Some("go") => matches!(
                node.kind(),
                "function_declaration"
                    | "method_declaration"
                    | "type_spec"
                    | "const_spec"
                    | "var_spec"
                    | "field_declaration"
            ),
            Some("swift") => matches!(
                node.kind(),
                "function_declaration"
                    | "class_declaration"
                    | "struct_declaration"
                    | "protocol_declaration"
                    | "enum_declaration"
                    | "typealias_declaration"
                    | "property_declaration"
            ),
            Some("typescript" | "tsx" | "javascript" | "jsx") => matches!(
                node.kind(),
                "function_declaration"
                    | "generator_function_declaration"
                    | "class_declaration"
                    | "abstract_class_declaration"
                    | "interface_declaration"
                    | "type_alias_declaration"
                    | "enum_declaration"
                    | "internal_module"
                    | "method_definition"
                    | "method_signature"
                    | "abstract_method_signature"
                    | "variable_declarator"
                    | "public_field_definition"
                    | "property_signature"
            ),
            _ => false,
        };
        if declaration {
            let mut cursor = node.walk();
            for name in node.children_by_field_name("name", &mut cursor) {
                self.add_node(name, source);
            }
        }
        if language == Some("rust") && node.kind() == "let_declaration" {
            if let Some(pattern) = node.child_by_field_name("pattern") {
                if pattern.kind() == "identifier" {
                    self.add_node(pattern, source);
                }
            }
        }
        if language == Some("rust") && node.kind() == "impl_item" {
            for field in ["type", "trait"] {
                if let Some(name) = node.child_by_field_name(field) {
                    self.add_type_head(name, source);
                }
            }
        }
    }

    fn add_node(&mut self, node: tree_sitter::Node<'_>, source: &str) {
        if let Ok(name) = node.utf8_text(source.as_bytes()) {
            self.add(node.start_position().row + 1, name);
        }
    }

    fn add_type_head(&mut self, node: tree_sitter::Node<'_>, source: &str) {
        match node.kind() {
            "identifier" | "type_identifier" => self.add_node(node, source),
            "generic_type" => {
                if let Some(name) = node.child_by_field_name("type") {
                    self.add_type_head(name, source);
                }
            }
            "scoped_type_identifier" | "scoped_identifier" => {
                if let Some(name) = node.child_by_field_name("name") {
                    self.add_type_head(name, source);
                }
            }
            "reference_type" => {
                if let Some(name) = node.child_by_field_name("type") {
                    self.add_type_head(name, source);
                }
            }
            _ => {}
        }
    }

    /// Query expressions are regexes, not literal symbol names. Match only the
    /// declared identifier so parameter/return-type mentions remain usages.
    pub(crate) fn is_declaration(&self, line: usize, query_patterns: &[String]) -> bool {
        let Some(names) = self.names.get(&line) else {
            return false;
        };
        if query_patterns.is_empty() {
            return true;
        }
        let mut cached = self.patterns.borrow_mut();
        if cached
            .as_ref()
            .is_none_or(|(patterns, _)| patterns != query_patterns)
        {
            *cached = Some((query_patterns.to_vec(), RegexSet::new(query_patterns).ok()));
        }
        let Some((_, Some(regex))) = cached.as_ref() else {
            return false;
        };
        names.iter().any(|name| regex.is_match(name))
    }

    fn add(&mut self, line: usize, name: &str) {
        let names = self.names.entry(line).or_default();
        if !names.iter().any(|existing| existing == name) {
            names.push(name.to_owned());
        }
    }
}

fn grammar(language: Option<&str>) -> Option<tree_sitter::Language> {
    Some(match language? {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "swift" => tree_sitter_swift::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" | "javascript" | "jsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        _ => return None,
    })
}

struct Token<'a> {
    text: &'a str,
    line: usize,
}
impl Token<'_> {
    fn identifier(&self) -> bool {
        self.text
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
    }
}

/// Bounded lexical fallback: discard comments and quoted text, retain row and
/// identifier boundaries. No declaration recognition is performed in strings.
fn tokenize(source: &str) -> Vec<Token<'_>> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut line) = (0, 1);
    while i < bytes.len() {
        let start = i;
        match bytes[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b if b.is_ascii_whitespace() => i += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'#' if bytes.get(i + 1) != Some(&b'[') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            quote @ (b'"' | 39 | 96) => {
                // Rust lifetimes have no closing apostrophe on this row.
                if quote == 39
                    && !bytes[i + 1..]
                        .iter()
                        .take_while(|b| **b != b'\n')
                        .any(|b| *b == 39)
                {
                    i += 1;
                    continue;
                }
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    if bytes[i] == b'\\' {
                        i = (i + 2).min(bytes.len());
                    } else if bytes[i] == quote {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
            }
            b if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 128 => {
                i += 1;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || matches!(bytes[i], b'_' | b'$')
                        || bytes[i] >= 128)
                {
                    i += 1;
                }
                out.push(Token {
                    text: &source[start..i],
                    line,
                });
            }
            _ => {
                i += 1;
                out.push(Token {
                    text: &source[start..i],
                    line,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(extension: &str, source: &str, query: &str, definitions: &[usize], usages: &[usize]) {
        let index = DeclarationIndex::new(Path::new(&format!("app.{extension}")), source);
        let patterns = vec![query.to_owned()];
        for &line in definitions {
            assert!(
                index.is_declaration(line, &patterns),
                "{extension}:{line} {query}"
            );
        }
        for &line in usages {
            assert!(
                !index.is_declaration(line, &patterns),
                "{extension}:{line} {query}"
            );
        }
    }
    #[test]
    fn declaration_names_not_body_spans_or_signature_mentions() {
        check(
            "rs",
            "trait Target {}\nfn caller() -> impl Target { todo!() }\n",
            "Target",
            &[1],
            &[2],
        );
        check("ts", "class C { target() {} }\n", "target", &[1], &[]);
        check(
            "rs",
            "struct Target;\nimpl Target {\n#[inline]\npub const fn target(\n value: Target\n) -> Target {\n target();\n}\n}\n",
            "Target",
            &[1, 2],
            &[4, 5, 6, 7],
        );
        check(
            "rs",
            "#[inline]\npub const fn\n target() {\n target();\n}\n",
            "target",
            &[3],
            &[1, 2, 4],
        );
        check(
            "py",
            "class Target:\n def target(self):\n  target()\n",
            "target",
            &[2],
            &[3],
        );
        check(
            "swift",
            "struct Thing {\n func target() {\n target()\n }\n}\n",
            "target",
            &[2],
            &[3],
        );
        check(
            "go",
            "package app\ntype Thing struct {}\nfunc (t Thing) target() {\n target()\n}\n",
            "target",
            &[3],
            &[4],
        );
    }
    #[test]
    fn js_arrows_methods_generators_and_regex_names() {
        for ext in ["ts", "js", "tsx", "jsx"] {
            check(
                ext,
                "const alpha = () => {};\nfunction* beta() {}\nclass C {\n alpha() {\n beta();\n }\n}\n",
                "alpha|beta",
                &[1, 2, 4],
                &[5],
            );
        }
        check(
            "rs",
            "fn target_extra() {}\nfn caller() { target(); }\n",
            "target.*",
            &[1],
            &[2],
        );
    }
    #[test]
    fn unsupported_fallback_ignores_comments_strings_and_calls() {
        check(
            "unknown",
            "// function target() {}\nconst text = \"function target() {}\";\nfunction target() {\n target();\n}\n",
            "target",
            &[3],
            &[1, 2, 4],
        );
    }
}
