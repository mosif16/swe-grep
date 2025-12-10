use std::collections::HashSet;

#[derive(Clone, Debug)]
pub(crate) struct ConceptExpansion {
    primary: Vec<String>,
    synonyms: Vec<String>,
    phrases: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConceptHitCounts {
    pub primary: usize,
    pub synonyms: usize,
    pub phrases: usize,
}

impl ConceptExpansion {
    pub(crate) fn primary(&self) -> &[String] {
        &self.primary
    }

    pub(crate) fn synonyms(&self) -> &[String] {
        &self.synonyms
    }

    pub(crate) fn phrases(&self) -> &[String] {
        &self.phrases
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

        for phrase in &self.phrases {
            if seen.insert(phrase.clone()) {
                terms.push(phrase.clone());
            }
        }

        terms
    }

    pub(crate) fn count_matches(&self, text: &str) -> ConceptHitCounts {
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
        let phrase_hits = self
            .phrases
            .iter()
            .filter(|token| haystack.contains(token.as_str()))
            .count();

        ConceptHitCounts {
            primary: primary_hits,
            synonyms: synonym_hits,
            phrases: phrase_hits,
        }
    }
}

pub(crate) fn expand_concept(phrase: &str) -> ConceptExpansion {
    let primary = tokenize(phrase);
    let mut synonyms: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = primary.iter().cloned().collect();
    let phrases = extract_phrases(phrase, &primary);

    for token in &primary {
        for synonym in synonyms_for(token) {
            let normalized = synonym.to_ascii_lowercase();
            if seen.insert(normalized.clone()) {
                synonyms.push(normalized);
            }
        }
    }

    ConceptExpansion {
        primary,
        synonyms,
        phrases,
    }
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

fn extract_phrases(raw: &str, tokens: &[String]) -> Vec<String> {
    let mut phrases: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let normalized = collapse_whitespace(raw);
    if normalized.split_whitespace().count() > 1 && seen.insert(normalized.clone()) {
        phrases.push(normalized);
    }

    // Sliding windows over tokens to capture common two and three word phrases.
    for window in tokens.windows(2) {
        let phrase = window.join(" ");
        if seen.insert(phrase.clone()) {
            phrases.push(phrase);
        }
    }
    for window in tokens.windows(3) {
        let phrase = window.join(" ");
        if seen.insert(phrase.clone()) {
            phrases.push(phrase);
        }
    }

    phrases
}

fn collapse_whitespace(value: &str) -> String {
    value
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

const PROGRAMMING_SYNONYMS: &[(&str, &[&str])] = &[
    (
        "rag",
        &[
            "retrieval",
            "embedding",
            "vector",
            "semantic",
            "index",
            "chunk",
            "context",
        ],
    ),
    (
        "auth",
        &[
            "authentication",
            "authenticate",
            "login",
            "logout",
            "session",
            "token",
            "jwt",
            "oauth",
            "credentials",
        ],
    ),
    (
        "authentication",
        &["auth", "login", "logout", "session", "token", "jwt"],
    ),
    (
        "login",
        &["auth", "authentication", "signin", "session", "oauth"],
    ),
    ("token", &["jwt", "bearer", "oauth", "auth", "session"]),
    ("jwt", &["token", "auth", "bearer"]),
    ("session", &["auth", "login", "authentication"]),
    (
        "cache",
        &[
            "caching",
            "memoize",
            "store",
            "persist",
            "warm",
            "invalidate",
            "ttl",
            "lru",
        ],
    ),
    (
        "caching",
        &[
            "cache",
            "memoize",
            "store",
            "persist",
            "warm",
            "invalidate",
            "ttl",
            "lru",
        ],
    ),
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
    use super::{ConceptExpansion, ConceptHitCounts, expand_concept};

    fn contains_term(expansion: &ConceptExpansion, value: &str) -> bool {
        expansion.synonyms().iter().any(|synonym| synonym == value)
    }

    #[test]
    fn expands_programming_synonyms() {
        let expansion = expand_concept("Auth flow");
        assert_eq!(
            expansion.primary(),
            &["auth".to_string(), "flow".to_string()]
        );
        assert!(contains_term(&expansion, "login"));
        assert!(contains_term(&expansion, "jwt"));
        assert!(contains_term(&expansion, "logout"));
    }

    #[test]
    fn counts_primary_and_synonym_hits() {
        let expansion = expand_concept("cache warming");
        let ConceptHitCounts {
            primary,
            synonyms,
            phrases,
        } = expansion.count_matches("memoize this cache warming pipeline");
        assert_eq!(primary, 2);
        assert!(synonyms >= 1);
        assert!(phrases >= 1);
    }

    #[test]
    fn captures_phrases_and_windows() {
        let expansion = expand_concept("RAG indexing pipeline");
        assert!(
            expansion
                .phrases()
                .contains(&"rag indexing pipeline".to_string())
        );
        assert!(expansion.phrases().contains(&"rag indexing".to_string()));
        assert!(expansion.synonyms().contains(&"embedding".to_string()));
    }
}
