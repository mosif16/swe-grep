use std::cmp::Ordering;
use std::collections::HashMap;

use strsim::normalized_levenshtein;

/// Split a string into identifier-like tokens (alphanumeric/underscore).
pub(crate) fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(current.clone());
            current.clear();
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    // Also capture Rust/Swift-style qualified names (Foo::bar::Baz) as a single token.
    tokens.extend(tokenize_qualified(text));
    tokens
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ScoredSuggestion {
    pub(crate) symbol: String,
    pub(crate) score: f32,
}

/// Rank candidate symbols by fuzzy similarity to the target and return the top scored results.
pub(crate) fn rank_scored_suggestions(
    target: &str,
    candidates: impl IntoIterator<Item = String>,
    limit: usize,
) -> Vec<ScoredSuggestion> {
    let target_lower = target.to_ascii_lowercase();
    let mut best_by_lower: HashMap<String, (f64, String)> = HashMap::new();

    for candidate in candidates {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        // Avoid echoing the exact same string back, but do keep case-only variants.
        if trimmed == target {
            continue;
        }

        let score = score_candidate(&target_lower, &lower);

        if score < 0.2 {
            continue;
        }
        match best_by_lower.get(&lower) {
            Some((existing, _)) if *existing >= score => {}
            _ => {
                best_by_lower.insert(lower, (score, trimmed.to_string()));
            }
        }
    }

    let mut scored: Vec<(f64, String)> = best_by_lower.into_values().collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.1.cmp(&b.1))
    });

    scored
        .into_iter()
        .take(limit)
        .map(|(score, value)| ScoredSuggestion {
            symbol: value,
            score: score as f32,
        })
        .collect()
}

fn tokenize_qualified(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        if !is_ident_start(chars[i]) {
            i += 1;
            continue;
        }

        let start = i;
        i += 1;
        while i < chars.len() && is_ident_continue(chars[i]) {
            i += 1;
        }

        let mut end = i;
        let mut has_qualifier = false;
        while i + 1 < chars.len() && chars[i] == ':' && chars[i + 1] == ':' {
            let qualifier_start = i;
            i += 2;
            if i >= chars.len() || !is_ident_start(chars[i]) {
                // Not a valid qualifier; rewind and stop extending.
                i = qualifier_start;
                break;
            }
            has_qualifier = true;
            i += 1;
            while i < chars.len() && is_ident_continue(chars[i]) {
                i += 1;
            }
            end = i;
        }

        if has_qualifier {
            tokens.push(chars[start..end].iter().collect());
        }
    }

    tokens
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn score_candidate(target_lower: &str, candidate_lower: &str) -> f64 {
    if candidate_lower == target_lower {
        return 1.0;
    }

    let mut score = normalized_levenshtein(target_lower, candidate_lower);

    if candidate_lower.starts_with(target_lower) {
        score += 0.25;
    } else if target_lower.starts_with(candidate_lower) {
        score += 0.15;
    } else if candidate_lower.contains(target_lower) {
        score += 0.1;
    } else if target_lower.contains(candidate_lower) {
        score += 0.05;
    }

    // Qualified name support: "::new" should strongly match "Foo::new".
    let target_suffix = target_lower.strip_prefix("::").unwrap_or(target_lower);
    if !target_suffix.is_empty() {
        let qualified_suffix = format!("::{target_suffix}");
        if candidate_lower.ends_with(&qualified_suffix) {
            score += 0.35;
        }
    }

    // Also compare the last qualified segment as a fallback signal.
    let target_last = target_lower.rsplit("::").next().unwrap_or(target_lower);
    let cand_last = candidate_lower
        .rsplit("::")
        .next()
        .unwrap_or(candidate_lower);
    if !target_last.is_empty() && !cand_last.is_empty() {
        score = score.max(0.92 * normalized_levenshtein(target_last, cand_last));
    }

    score.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{rank_scored_suggestions, tokenize};

    #[test]
    fn tokenizes_identifiers() {
        let tokens = tokenize("fn SearchEngine::run_cycle(data)");
        assert!(tokens.contains(&"SearchEngine".to_string()));
        assert!(tokens.contains(&"run_cycle".to_string()));
        assert!(tokens.contains(&"SearchEngine::run_cycle".to_string()));
    }

    #[test]
    fn ranks_by_similarity() {
        let candidates = vec![
            "SearchEngine".to_string(),
            "SearchConfig".to_string(),
            "Searcher".to_string(),
        ];
        let ranked = rank_scored_suggestions("SearchEngin", candidates, 3);
        assert_eq!(ranked.first().unwrap().symbol, "SearchEngine");
    }

    #[test]
    fn keeps_case_only_variants() {
        let candidates = vec!["AuthService".to_string(), "authservice".to_string()];
        let ranked = rank_scored_suggestions("authservice", candidates, 5);
        assert_eq!(ranked.first().unwrap().symbol, "AuthService");
        assert!((ranked.first().unwrap().score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn prefers_qualified_suffix_matches() {
        let candidates = vec![
            "Foo::new".to_string(),
            "Bar::new".to_string(),
            "new".to_string(),
        ];
        let ranked = rank_scored_suggestions("::new", candidates, 3);
        assert!(ranked.iter().any(|s| s.symbol.ends_with("::new")));
        assert!(ranked.first().unwrap().score > 0.8);
    }
}
