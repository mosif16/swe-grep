use std::collections::HashSet;

use crate::language;

#[derive(Debug)]
pub(crate) struct QueryRewriter {
    symbol: String,
    languages: Vec<String>,
}

impl QueryRewriter {
    pub(crate) fn for_symbol(symbol: &str, languages: &[String]) -> Self {
        Self {
            symbol: symbol.to_string(),
            languages: languages.iter().cloned().collect(),
        }
    }

    pub(crate) fn build(&self) -> Vec<String> {
        let s = self.symbol.trim();
        if s.is_empty() {
            return Vec::new();
        }
        let type_hint = self.derive_type_hint();

        let mut queries = vec![
            Self::escape_literal(s),
            Self::escape_literal(&format!("{s} {type_hint}")),
            Self::escape_literal(&format!("{s} error")),
            Self::escape_literal(&format!("{type_hint}.{s}")),
        ];

        for lang in &self.languages {
            for pattern in language::query_patterns_for(lang, s) {
                queries.push(Self::escape_literal(&pattern));
            }
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
