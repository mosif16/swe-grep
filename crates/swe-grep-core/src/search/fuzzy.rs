use std::cmp::Ordering;
use std::collections::HashSet;

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

    tokens
}

/// Rank candidate symbols by fuzzy similarity to the target and return the top results.
pub(crate) fn rank_suggestions(
    target: &str,
    candidates: impl IntoIterator<Item = String>,
    limit: usize,
) -> Vec<String> {
    let target_lower = target.to_ascii_lowercase();
    let mut seen: HashSet<String> = HashSet::new();
    let mut scored: Vec<(f64, String)> = Vec::new();

    for candidate in candidates {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower == target_lower || !seen.insert(lower.clone()) {
            continue;
        }

        let mut score = normalized_levenshtein(&target_lower, &lower);
        if lower.contains(&target_lower) {
            score += 0.1;
        } else if target_lower.contains(&lower) {
            score += 0.05;
        }

        if score < 0.2 {
            continue;
        }
        scored.push((score, trimmed.to_string()));
    }

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.1.cmp(&b.1))
    });

    scored
        .into_iter()
        .map(|(_, value)| value)
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{rank_suggestions, tokenize};

    #[test]
    fn tokenizes_identifiers() {
        let tokens = tokenize("fn SearchEngine::run_cycle(data)");
        assert!(tokens.contains(&"SearchEngine".to_string()));
        assert!(tokens.contains(&"run_cycle".to_string()));
    }

    #[test]
    fn ranks_by_similarity() {
        let candidates = vec![
            "SearchEngine".to_string(),
            "SearchConfig".to_string(),
            "Searcher".to_string(),
        ];
        let ranked = rank_suggestions("SearchEngin", candidates, 3);
        assert_eq!(ranked.first().unwrap(), "SearchEngine");
    }
}
