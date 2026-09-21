//! Deterministic lexical scorer (Phase 8, 8-R7, retrieval-design §2).
//!
//! Ordering signal only: it NEVER filters, NEVER persists a score, and
//! NEVER affects memory truth. With empty terms every score is zero and
//! ordering reduces to the caller's base order (D24 for context, id order
//! for proposals) — dormancy is structural, not a flag.
//!
//! Tokenize: lowercase, split on non-alphanumeric, drop tokens shorter
//! than 3 characters and a fixed stopword list.
//! Score: Σ over distinct matched terms of `1 / (1 + freq(term))` where
//! `freq` counts corpus records containing the term. Stable
//! score-descending pass; ties keep the input order (callers pre-sort by
//! D24 or id, so ties are deterministic).

use super::record::MemoryRecord;

/// Fixed stopword list (tested). Small on purpose: only the most common
/// English function words, so domain terms always survive.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "have", "has", "had", "was", "were",
    "are", "our", "you", "your", "its", "not", "but", "all", "can", "will", "would", "should",
    "could", "they", "them", "their", "his", "her", "she", "him", "who", "whom", "which", "what",
    "when", "where", "how", "why", "than", "then", "there", "here", "into", "over", "under",
    "about", "between", "through", "during", "such", "other", "more", "most", "some", "any",
    "each", "few", "own", "same", "too", "very", "just", "also", "use", "used", "using",
];

/// Tokenize text for scoring. Deterministic; duplicates preserved here
/// (callers dedup where the spec requires distinct terms).
pub fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() >= 3 && !STOPWORDS.contains(token))
        .map(str::to_owned)
        .collect()
}

/// Score one text against query terms: sum over DISTINCT matched terms of
/// `1 / (1 + freq)`. `freq` counts corpus records containing the term.
/// Empty terms → 0.0 (dormant).
pub fn score(text: &str, terms: &[String], freq: &dyn Fn(&str) -> usize) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let present = tokenize(text);
    let mut seen: Vec<&str> = Vec::new();
    let mut total = 0.0;
    for term in terms {
        if seen.iter().any(|seen| *seen == term) {
            continue;
        }
        seen.push(term);
        if present.iter().any(|token| token == term) {
            total += 1.0 / (1.0 + freq(term) as f64);
        }
    }
    total
}

/// Corpus frequency helper: how many record texts contain `term` as a
/// token. O(records × tokens); negligible at our scale (gate: revisit
/// FTS5 only on sustained >50 ms — retrieval-design §2).
pub fn frequency(corpus: &[String], term: &str) -> usize {
    corpus
        .iter()
        .filter(|text| tokenize(text).iter().any(|token| token == term))
        .count()
}

/// Stable score-descending pass over D24-ordered (or otherwise
/// pre-ordered) memory records. Equal scores keep input order, so empty
/// terms reproduce the input byte-identically (tested).
pub fn order_records(records: &mut [MemoryRecord], terms: &[String]) {
    if terms.is_empty() {
        return;
    }
    let corpus: Vec<String> = records
        .iter()
        .map(|record| record.content.clone())
        .collect();
    let mut scored: Vec<(f64, usize)> = records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let score = score(&record.content, terms, &|term| frequency(&corpus, term));
            (score, index)
        })
        .collect();
    // Stable: equal scores keep the pre-existing (D24) order.
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let order: Vec<usize> = scored.into_iter().map(|(_, index)| index).collect();
    let mut sorted: Vec<MemoryRecord> = Vec::with_capacity(records.len());
    for index in order {
        sorted.push(records[index].clone());
    }
    records.clone_from_slice(&sorted);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory::record::{Kind, Scope, Source, Status};

    fn record(id: &str, content: &str) -> MemoryRecord {
        MemoryRecord {
            id: id.to_owned(),
            key: None,
            kind: Kind::Fact,
            scope: Scope::User,
            content: content.to_owned(),
            source: Source::User,
            status: Status::Active,
            pinned: false,
            created_at: "2026-09-18T10:00:00Z".to_owned(),
            updated_at: "2026-09-18T10:00:00Z".to_owned(),
            session_id: "ses".to_owned(),
            source_ref: None,
            quote: None,
            method: None,
        }
    }

    #[test]
    fn tokenize_lowercases_and_filters() {
        assert_eq!(
            tokenize("Prefer Rust, always!"),
            vec!["prefer", "rust", "always"]
        );
        // <3 chars and stopwords dropped ("db" is two chars: dropped).
        assert_eq!(tokenize("I go to the database"), vec!["database"]);
        assert!(tokenize("the and for").is_empty());
    }

    #[test]
    fn score_is_deterministic_and_dormant_empty() {
        let corpus = vec!["prefer rust".to_owned(), "rust compiler".to_owned()];
        let freq = |term: &str| frequency(&corpus, term);
        let terms = vec!["rust".to_owned(), "compiler".to_owned()];
        let first = score("Rust compiler flags", &terms, &freq);
        let second = score("Rust compiler flags", &terms, &freq);
        assert_eq!(first, second);
        assert_eq!(score("anything", &[], &freq), 0.0);
        // Distinct terms only: repeated query term counts once.
        assert_eq!(
            score("rust", &["rust".to_owned(), "rust".to_owned()], &freq),
            score("rust", &["rust".to_owned()], &freq)
        );
        // Rarer term weighs more: compiler (1/2) > rust (2/3 → 1/3).
        assert!(
            score("compiler", &terms, &freq) > score("rust", &terms, &freq),
            "rarity weighting"
        );
    }

    #[test]
    fn order_records_empty_terms_is_identity() {
        let mut records = vec![record("owt_1_1_2", "second"), record("owt_1_1_1", "first")];
        let before = records.clone();
        order_records(&mut records, &[]);
        assert_eq!(records, before);
    }

    #[test]
    fn order_records_ranks_by_score_stably() {
        let mut records = vec![
            record("owt_1_1_1", "unrelated weather note"),
            record("owt_1_1_2", "rust compiler flags"),
            record("owt_1_1_3", "rust edition"),
        ];
        order_records(&mut records, &["compiler".to_owned(), "rust".to_owned()]);
        let ids: Vec<&str> = records.iter().map(|r| r.id.as_str()).collect();
        // Both rust rows beat the weather row; compiler row first (rarer).
        assert_eq!(ids[0], "owt_1_1_2");
        assert_eq!(ids[1], "owt_1_1_3");
        assert_eq!(ids[2], "owt_1_1_1");
    }
}
