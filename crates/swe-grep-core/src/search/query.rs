use std::collections::HashSet;

use super::concept_expansion::ConceptExpansion;
use crate::language;

#[derive(Debug, Clone, Copy)]
pub(crate) enum PatternMode {
    Literal,
    LiteralText,
    LiteralString,
    Regex,
    RawRegex,
    Wildcard,
}

#[derive(Debug)]
enum QueryMode {
    Symbol { pattern_mode: PatternMode },
    Concept { expansion: ConceptExpansion },
}

#[derive(Debug)]
pub(crate) struct QueryRewriter {
    symbol: String,
    languages: Vec<String>,
    mode: QueryMode,
    ignore_case: bool,
    attribute: bool,
}

impl QueryRewriter {
    pub(crate) fn for_symbol(
        symbol: &str,
        languages: &[String],
        pattern_mode: PatternMode,
        ignore_case: bool,
        attribute: bool,
    ) -> Self {
        Self {
            symbol: symbol.to_string(),
            languages: languages.iter().cloned().collect(),
            mode: QueryMode::Symbol { pattern_mode },
            ignore_case,
            attribute,
        }
    }

    pub(crate) fn for_concept(
        concept: &str,
        languages: &[String],
        ignore_case: bool,
        expansion: ConceptExpansion,
    ) -> Self {
        Self {
            symbol: concept.to_string(),
            languages: languages.iter().cloned().collect(),
            mode: QueryMode::Concept { expansion },
            ignore_case,
            attribute: false,
        }
    }

    pub(crate) fn build(&self) -> Vec<String> {
        let s = self.symbol.trim();
        if s.is_empty() {
            return Vec::new();
        }
        match &self.mode {
            QueryMode::Symbol { pattern_mode } => self.build_symbol_queries(*pattern_mode),
            QueryMode::Concept { expansion } => self.build_concept_queries(expansion),
        }
    }

    fn build_symbol_queries(&self, pattern_mode: PatternMode) -> Vec<String> {
        let s = self.symbol.trim();
        if s.is_empty() {
            return Vec::new();
        }
        let type_hint = self.derive_type_hint();

        let queries = match pattern_mode {
            PatternMode::Literal => vec![
                Self::escape_literal(s),
                Self::escape_literal(&format!("{s} {type_hint}")),
                Self::escape_literal(&format!("{s} error")),
                Self::escape_literal(&format!("{type_hint}.{s}")),
            ],
            PatternMode::LiteralText => vec![Self::escape_literal(s)],
            PatternMode::LiteralString => {
                let escaped = Self::escape_literal(s);
                let mut patterns = vec![escaped.clone(), format!("(?i){escaped}")];
                for quote in ['"', '\'', '`'] {
                    patterns.push(format!("{quote}{escaped}{quote}"));
                    patterns.push(format!("(?i){quote}{escaped}{quote}"));
                }
                patterns
            }
            PatternMode::Regex => {
                let mut patterns = Vec::new();
                patterns.push(s.to_string());
                if !s.starts_with("(?i)") {
                    patterns.push(format!("(?i){s}"));
                }
                patterns
            }
            PatternMode::RawRegex => vec![s.to_string()],
            PatternMode::Wildcard => {
                let regexified = wildcard_to_regex(s);
                let mut patterns = vec![regexified.clone()];
                if !regexified.starts_with("(?i)") {
                    patterns.push(format!("(?i){regexified}"));
                }
                patterns
            }
        };

        let mut queries = self.enforce_case_insensitive(queries);

        if !matches!(
            pattern_mode,
            PatternMode::LiteralString | PatternMode::LiteralText | PatternMode::RawRegex
        ) {
            for lang in &self.languages {
                for pattern in language::query_patterns_for(lang, s) {
                    let rendered = match pattern_mode {
                        PatternMode::Literal => Self::escape_literal(&pattern),
                        PatternMode::Regex => pattern,
                        PatternMode::Wildcard => wildcard_to_regex(&pattern),
                        PatternMode::LiteralString | PatternMode::LiteralText => {
                            Self::escape_literal(&pattern)
                        }
                        PatternMode::RawRegex => pattern,
                    };
                    queries.push(rendered);
                }

                if self.attribute {
                    for pattern in language::attribute_patterns_for(lang, s) {
                        let rendered = match pattern_mode {
                            PatternMode::Literal => Self::escape_literal(&pattern),
                            PatternMode::Regex => pattern,
                            PatternMode::Wildcard => wildcard_to_regex(&pattern),
                            PatternMode::LiteralString | PatternMode::LiteralText => {
                                Self::escape_literal(&pattern)
                            }
                            PatternMode::RawRegex => pattern,
                        };
                        queries.push(rendered);
                    }
                }
            }
        }

        dedup_queries(queries)
    }

    fn enforce_case_insensitive(&self, patterns: Vec<String>) -> Vec<String> {
        if !self.ignore_case {
            return patterns;
        }
        patterns
            .into_iter()
            .map(|pattern| {
                if pattern.starts_with("(?i)") {
                    pattern
                } else {
                    format!("(?i){pattern}")
                }
            })
            .collect()
    }

    fn build_concept_queries(&self, expansion: &ConceptExpansion) -> Vec<String> {
        let primary = expansion.primary();
        let phrases = expansion.phrases();
        let all_terms = expansion.all_terms();

        if primary.is_empty() && all_terms.is_empty() {
            return Vec::new();
        }
        let mut queries: Vec<String> = Vec::new();

        for phrase in phrases {
            if phrase.contains(' ') {
                let spaced = phrase.replace(' ', ".*");
                queries.push(format!("(?i){}", Self::escape_literal(&spaced)));
            } else {
                queries.push(format!("(?i){}", Self::escape_literal(phrase)));
            }
        }

        let phrase_terms = if primary.is_empty() {
            all_terms.clone()
        } else {
            primary.to_vec()
        };

        if !phrase_terms.is_empty() {
            let collapsed = phrase_terms.join(".*");
            queries.push(format!("(?i){}", Self::escape_literal(&collapsed)));
        }

        // Build sliding window pairs to capture nearby phrases.
        for window in phrase_terms.windows(2) {
            if let [first, second] = window {
                let pattern = format!(
                    "(?i){}.*{}",
                    Self::escape_literal(first),
                    Self::escape_literal(second)
                );
                queries.push(pattern);
            }
        }

        // Single-token fallbacks
        for token in all_terms {
            queries.push(format!("(?i){}", Self::escape_literal(&token)));
        }

        dedup_queries(queries)
    }

    fn derive_type_hint(&self) -> String {
        let s = self.symbol.trim();
        if s.is_empty() {
            return "value".to_string();
        }
        if let Some(part) = s.rsplit([':', '_', '.']).next() {
            if s.contains('_') {
                return capitalize(part);
            }
        }
        if let Some(index) = s
            .char_indices()
            .filter(|(_, c)| c.is_uppercase())
            .map(|(i, _)| i)
            .last()
        {
            return s[index..].to_string();
        }

        capitalize(s)
    }

    fn escape_literal(value: &str) -> String {
        let mut escaped = String::with_capacity(value.len());
        for ch in value.chars() {
            match ch {
                '\\' | '.' | '+' | '*' | '?' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}'
                | '|' => {
                    escaped.push('\\');
                    escaped.push(ch);
                }
                _ => escaped.push(ch),
            }
        }
        escaped
    }
}

fn capitalize(segment: &str) -> String {
    let mut chars = segment.chars();
    if let Some(first) = chars.next() {
        let mut result = String::new();
        result.push(first.to_ascii_uppercase());
        result.extend(chars);
        return result;
    }
    segment.to_string()
}

fn dedup_queries<I>(queries: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for query in queries {
        if seen.insert(query.clone()) {
            deduped.push(query);
        }
    }
    deduped
}

fn wildcard_to_regex(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let mut pattern = String::new();
    pattern.push_str("(?i)");
    for ch in value.chars() {
        match ch {
            '*' => pattern.push_str(".*"),
            '?' => pattern.push('.'),
            '\\' | '.' | '+' | '[' | ']' | '{' | '}' | '(' | ')' | '^' | '$' | '|' => {
                pattern.push('\\');
                pattern.push(ch);
            }
            _ => pattern.push(ch),
        }
    }
    pattern
}
