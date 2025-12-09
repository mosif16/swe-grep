use std::collections::HashSet;

#[derive(Clone, Debug)]
pub(crate) struct ConceptExpansion {
    primary: Vec<String>,
    synonyms: Vec<String>,
}

impl ConceptExpansion {
    pub(crate) fn primary(&self) -> &[String] {
        &self.primary
    }

    pub(crate) fn synonyms(&self) -> &[String] {
        &self.synonyms
    }

    pub(crate) fn all_terms(&self) -> Vec<String> {
        let mut terms = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for token in &self.primary {
            if seen.insert(token.clone()) {
                terms.push(token.clone());
            }
        }

        for token in &self.synonyms {
            if seen.insert(token.clone()) {
                terms.push(token.clone());
            }
        }

        terms
    }

    pub(crate) fn count_matches(&self, text: &str) -> (usize, usize) {
        let haystack = text.to_ascii_lowercase();

        let primary_hits = self
            .primary
            .iter()
            .filter(|token| haystack.contains(token.as_str()))
            .count();
        let synonym_hits = self
            .synonyms
            .iter()
            .filter(|token| haystack.contains(token.as_str()))
            .count();

        (primary_hits, synonym_hits)
    }
}

pub(crate) fn expand_concept(phrase: &str) -> ConceptExpansion {
    let primary = tokenize(phrase);
    let mut synonyms: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = primary.iter().cloned().collect();

    for token in &primary {
        for synonym in synonyms_for(token) {
            let normalized = synonym.to_ascii_lowercase();
            if seen.insert(normalized.clone()) {
                synonyms.push(normalized);
            }
        }
    }

    ConceptExpansion { primary, synonyms }
}

fn tokenize(phrase: &str) -> Vec<String> {
    let mut tokens: Vec<String> = phrase
        .split(|ch: char| ch.is_whitespace() || ch == '-' || ch == '_' || ch == '.')
        .filter_map(|raw| {
            let normalized = raw.trim().to_ascii_lowercase();
            if normalized.len() <= 1 {
                None
            } else {
                Some(normalized)
            }
        })
        .collect();
    tokens.dedup();
    tokens
}

const PROGRAMMING_SYNONYMS: &[(&str, &[&str])] = &[
    ("rag", &["retrieval", "embedding", "vector", "semantic"]),
    (
        "auth",
        &["authentication", "login", "session", "token", "jwt", "oauth"],
    ),
    ("authentication", &["auth", "login", "session", "token", "jwt"]),
    ("login", &["auth", "authentication", "signin", "session"]),
    ("token", &["jwt", "bearer", "oauth", "auth"]),
    ("jwt", &["token", "auth"]),
    ("session", &["auth", "login", "authentication"]),
    ("cache", &["caching", "memoize", "store", "persist"]),
    ("caching", &["cache", "memoize", "store", "persist"]),
    ("error", &["exception", "failure", "catch", "handle"]),
    ("queue", &["message", "worker", "job"]),
    ("db", &["database", "sql", "postgres", "mysql"]),
    ("database", &["db", "sql", "postgres", "mysql"]),
];

fn synonyms_for(token: &str) -> &'static [&'static str] {
    let lower = token.to_ascii_lowercase();
    PROGRAMMING_SYNONYMS
        .iter()
        .find_map(|(root, synonyms)| {
            if root.eq_ignore_ascii_case(&lower) {
                Some(*synonyms)
            } else {
                None
            }
        })
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::{expand_concept, ConceptExpansion};

    fn contains_term(expansion: &ConceptExpansion, value: &str) -> bool {
        expansion
            .synonyms()
            .iter()
            .any(|synonym| synonym == value)
    }

    #[test]
    fn expands_programming_synonyms() {
        let expansion = expand_concept("Auth flow");
        assert_eq!(expansion.primary(), &["auth".to_string(), "flow".to_string()]);
        assert!(contains_term(&expansion, "login"));
        assert!(contains_term(&expansion, "jwt"));
    }

    #[test]
    fn counts_primary_and_synonym_hits() {
        let expansion = expand_concept("cache warming");
        let (primary_hits, synonym_hits) =
            expansion.count_matches("memoize this cache warming pipeline");
        assert_eq!(primary_hits, 2);
        assert!(synonym_hits >= 1);
    }
}
