//! Deterministic rule-based proposal generator (Phase 8, 8-R1–R3,
//! intelligence-design §1–§2, architecture §3).
//!
//! Pure function `history → proposals`: user-role sentences in, typed
//! drafts out. It never writes, deletes, injects, networks, or calls a
//! model. Conservative by design — when uncertain, no proposal.
//!
//! Pipeline per sentence: candidate filters → rule match → secret
//! screen (D23, drop refused AND warned) → tombstone pre-check →
//! store normalized-dedup → batch dedup → draft.

use super::record::{normalize_content, Kind, Scope};
use super::secret::scan;
use super::secret::SecretVerdict;

/// One generated candidate (becomes a `Proposal` on queue admission).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftProposal {
    pub text: String,
    pub kind: Kind,
    pub scope: Scope,
    pub quote: String,
    pub rule: String,
    pub session_id: String,
    pub needs_review: bool,
}

/// Read-only store view the generator dedups against.
pub struct StoreView<'a> {
    /// Whole ACTIVE corpus (both scopes); scope matching is per-proposal.
    pub active: &'a [super::record::MemoryRecord],
    /// Tombstone pre-check (kind + scope + normalized content).
    pub tombstone_hit: &'a dyn Fn(Kind, Scope, &str) -> bool,
}

/// Drop counters (secret/poison counts are logged without content).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GenStats {
    pub sentences: usize,
    pub proposed: usize,
    pub dropped_filter: usize,
    pub dropped_secret: usize,
    pub dropped_warned: usize,
    pub dropped_tombstone: usize,
    pub dropped_duplicate: usize,
}

/// Extract user-role texts from history message objects (`type ==
/// "user"`). Assistant/tool/shell/system content is structurally
/// excluded (8-R2) — never model output, never tool output, never files.
/// Command echoes (`/memory …`) are skipped (defense in depth: the
/// server history never contains them, mock blocks might).
pub fn user_texts(messages: &[serde_json::Value]) -> Vec<String> {
    messages
        .iter()
        .filter(|message| message.get("type").and_then(|t| t.as_str()) == Some("user"))
        .filter_map(|message| message.get("text").and_then(|t| t.as_str()))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .filter(|text| {
            !(text == &"/memory"
                || text == &"/mem"
                || text.starts_with("/memory ")
                || text.starts_with("/mem "))
        })
        .map(str::to_owned)
        .collect()
}

/// Split text into candidate sentences (deterministic): newlines always
/// split; sentence punctuation splits before whitespace + uppercase/end.
pub fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    for paragraph in text.split('\n') {
        let mut start = 0;
        let chars: Vec<char> = paragraph.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let is_end = matches!(chars[i], '.' | '!' | '?' | '…');
            let next = chars.get(i + 1);
            let boundary = is_end
                && match next {
                    None => true,
                    Some(' ') | Some('\t') => {
                        let rest: String = chars[i + 2..].iter().collect();
                        let rest = rest.trim_start();
                        rest.is_empty()
                            || rest
                                .chars()
                                .next()
                                .is_some_and(|c| c.is_uppercase() || c.is_numeric())
                    }
                    _ => false,
                };
            if boundary {
                let sentence: String = chars[start..=i].iter().collect();
                if !sentence.trim().is_empty() {
                    sentences.push(sentence.trim().to_owned());
                }
                start = i + 1;
            }
            i += 1;
        }
        let tail: String = chars[start..].iter().collect();
        if !tail.trim().is_empty() {
            sentences.push(tail.trim().to_owned());
        }
    }
    sentences
}

/// Strip bullet/quote/number prefixes so rules see the statement.
fn strip_bullet(sentence: &str) -> &str {
    let mut text = sentence.trim_start();
    for prefix in ["> ", "» ", "- ", "* ", "+ "] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.trim_start();
            break;
        }
    }
    // "1. ", "12) " ordered-list prefixes.
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i < bytes.len() && (bytes[i] == b'.' || bytes[i] == b')') {
        let rest = text[i + 1..].trim_start();
        if !rest.is_empty() {
            text = rest;
        }
    }
    text
}

fn word_count(sentence: &str) -> usize {
    sentence.split_whitespace().count()
}

/// Negative candidate filters (worthiness DO-NOT-KEEP + noise). True =
/// reject the sentence.
fn filtered(sentence: &str) -> bool {
    let lower = sentence.to_lowercase();
    // Questions.
    if sentence.trim_end().ends_with('?') {
        return true;
    }
    // Code blocks, prompts, quotes-as-code, indented code.
    if sentence.contains("```")
        || lower.starts_with("$ ")
        || lower.starts_with("# ")
        || lower.starts_with("> ")
        || sentence.starts_with("    ")
    {
        return true;
    }
    // Diffs.
    if lower.starts_with("+++")
        || lower.starts_with("---")
        || lower.starts_with("@@")
        || lower.starts_with("diff --git")
        || sentence.contains("@@")
    {
        return true;
    }
    // Error text.
    for token in [
        "error",
        "failed",
        "failure",
        "exception",
        "traceback",
        "panic",
        "stack trace",
    ] {
        if lower.contains(token) {
            return true;
        }
    }
    // URLs/keys.
    if lower.contains("://") || lower.contains("www.") {
        return true;
    }
    // Overly short content.
    if word_count(sentence) < 3 {
        return true;
    }
    // Session-local references.
    for anchor in [
        "this error",
        "that error",
        "right now",
        "right here",
        "this session",
        "today",
        "yesterday",
        "tomorrow",
    ] {
        if lower.contains(anchor) {
            return true;
        }
    }
    // Ephemeral task state.
    if lower.contains("todo") || lower == "wip" || lower.contains(" wip ") {
        return true;
    }
    for verb in ["restart", "reboot", "reload", "kill", "bounce"] {
        let words: Vec<&str> = lower.split_whitespace().collect();
        for window in words.windows(3) {
            if window[0] == verb && window[2].chars().any(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

fn first_word_is(sentence: &str, words: &[&str]) -> bool {
    let lower = sentence.to_lowercase();
    let first: String = lower.split_whitespace().take(1).collect();
    let first = first.trim_matches(|c: char| !c.is_alphanumeric());
    words.contains(&first)
}

fn contains_word(sentence: &str, word: &str) -> bool {
    sentence
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|token| token == word)
}

fn contains_phrase(lower: &str, phrase: &str) -> bool {
    lower.contains(phrase)
}

/// Rule match result before scope assignment.
struct RuleHit {
    kind: Kind,
    rule: &'static str,
    needs_review: bool,
    project_candidate: bool,
}

/// Try the KEEP rules (no review flag), then the CONFIRM-carefully
/// patterns (review flag). Returns None when nothing fires.
fn match_rule(sentence: &str) -> Option<RuleHit> {
    // imperative-v1: ^(always|never|prefer|use|avoid|remember), plus
    // first-person-plural tooling conventions ("we use/prefer/avoid X").
    if first_word_is(sentence, &["always", "never", "prefer", "use", "avoid"]) {
        return Some(RuleHit {
            kind: Kind::Preference,
            rule: "imperative-v1",
            needs_review: false,
            project_candidate: false,
        });
    }
    {
        let lower = sentence.to_lowercase();
        let words: Vec<&str> = lower.split_whitespace().collect();
        if matches!(words.as_slice(), ["we", verb, ..] if ["use", "prefer", "avoid"].contains(verb))
        {
            return Some(RuleHit {
                kind: Kind::Preference,
                rule: "imperative-v1",
                needs_review: false,
                project_candidate: false,
            });
        }
    }
    if first_word_is(sentence, &["remember"]) {
        return Some(RuleHit {
            kind: Kind::Fact,
            rule: "imperative-v1",
            needs_review: false,
            project_candidate: true,
        });
    }
    let lower = sentence.to_lowercase();
    // preference-v1: first-person preference statements.
    for verb in ["prefer", "like", "love", "dislike", "hate", "enjoy"] {
        if contains_phrase(&lower, &format!("i {verb} "))
            || contains_phrase(&lower, &format!("i {verb}	"))
            || lower.trim_end().ends_with(&format!("i {verb}"))
        {
            return Some(RuleHit {
                kind: Kind::Preference,
                rule: "preference-v1",
                needs_review: false,
                project_candidate: false,
            });
        }
    }
    // constraint-v1: must/only/required-style project-context statements.
    if contains_word(sentence, "must")
        || contains_word(sentence, "only")
        || contains_word(sentence, "required")
        || contains_word(sentence, "requires")
        || contains_word(sentence, "mandatory")
        || contains_word(sentence, "forbidden")
        || contains_word(sentence, "prohibited")
    {
        return Some(RuleHit {
            kind: Kind::Fact,
            rule: "constraint-v1",
            needs_review: false,
            project_candidate: true,
        });
    }
    // CONFIRM-carefully: goals, relationships, qualified statements.
    let goal = [
        "planning to",
        "plan to",
        "plans to",
        "migrating to",
        "migrate to",
        "goal is",
        "aim to",
        "aiming to",
    ]
    .iter()
    .any(|pattern| contains_phrase(&lower, pattern));
    let relation = [
        "owns ",
        "owner of",
        "responsible for",
        "maintains ",
        "maintainer of",
    ]
    .iter()
    .any(|pattern| contains_phrase(&lower, pattern));
    let qualified = [
        "usually",
        "often",
        "typically",
        "generally",
        "tends to",
        "tend to",
    ]
    .iter()
    .any(|pattern| contains_phrase(&lower, pattern));
    if goal || relation || qualified {
        return Some(RuleHit {
            kind: Kind::Fact,
            rule: if goal {
                "goal-v1"
            } else if relation {
                "relation-v1"
            } else {
                "qualified-v1"
            },
            needs_review: true,
            project_candidate: false,
        });
    }
    None
}

/// Generate proposal drafts from user texts. Deterministic: same inputs
/// → identical drafts in input order (no timestamps here; the triage
/// layer stamps `created_at` at queue admission).
pub fn generate(
    texts: &[String],
    session_id: &str,
    project_rooted: bool,
    view: &StoreView<'_>,
) -> (Vec<DraftProposal>, GenStats) {
    let mut stats = GenStats::default();
    let mut drafts = Vec::new();
    let mut batch_hashes = std::collections::HashSet::new();
    for text in texts {
        for sentence in split_sentences(text) {
            stats.sentences += 1;
            let sentence = strip_bullet(&sentence).trim().to_owned();
            if sentence.chars().count() > super::record::MAX_CONTENT_CHARS {
                stats.dropped_filter += 1;
                continue;
            }
            if filtered(&sentence) {
                stats.dropped_filter += 1;
                continue;
            }
            let Some(hit) = match_rule(&sentence) else {
                stats.dropped_filter += 1;
                continue;
            };
            // Security screen BEFORE anything visible (D23 unchanged;
            // refused AND warned both drop — proposals stay cleaner than
            // explicit remembers; counts only, never content, in logs).
            match scan(&sentence) {
                SecretVerdict::Refused => {
                    stats.dropped_secret += 1;
                    continue;
                }
                SecretVerdict::Warn(_) => {
                    stats.dropped_warned += 1;
                    continue;
                }
                SecretVerdict::None => {}
            }
            let scope = if hit.project_candidate && project_rooted {
                Scope::Project
            } else {
                Scope::User
            };
            // Tombstone pre-check: forgotten stays forgotten.
            if (view.tombstone_hit)(hit.kind, scope, &sentence) {
                stats.dropped_tombstone += 1;
                continue;
            }
            // Normalized dedup vs ACTIVE store (same kind + scope).
            let duplicate = view.active.iter().any(|record| {
                record.kind == hit.kind
                    && record.scope == scope
                    && normalize_content(&record.content) == normalize_content(&sentence)
            });
            if duplicate {
                stats.dropped_duplicate += 1;
                continue;
            }
            // Batch dedup within this run.
            let identity = super::tombstone::canonical_hash(hit.kind, scope, &sentence);
            if !batch_hashes.insert(identity) {
                stats.dropped_duplicate += 1;
                continue;
            }
            let quote = sentence
                .chars()
                .take(super::record::MAX_QUOTE_CHARS)
                .collect();
            drafts.push(DraftProposal {
                text: sentence.clone(),
                kind: hit.kind,
                scope,
                quote,
                rule: hit.rule.to_owned(),
                session_id: session_id.to_owned(),
                needs_review: hit.needs_review,
            });
            stats.proposed += 1;
        }
    }
    (drafts, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory::record::MemoryRecord;

    fn view() -> StoreView<'static> {
        StoreView {
            active: NO_ACTIVE,
            tombstone_hit: &no_tombstone,
        }
    }

    static NO_ACTIVE: &[MemoryRecord] = &[];

    fn no_tombstone(_: Kind, _: Scope, _: &str) -> bool {
        false
    }

    fn propose(text: &str) -> Vec<DraftProposal> {
        let view = view();
        let (drafts, _) = generate(&[text.to_owned()], "ses_1", false, &view);
        drafts
    }

    #[test]
    fn user_texts_filters_roles_and_commands() {
        let messages = vec![
            serde_json::json!({"type": "user", "text": "  hello  "}),
            serde_json::json!({"type": "assistant", "text": "hi"}),
            serde_json::json!({"type": "tool", "text": "output"}),
            serde_json::json!({"type": "user", "text": "/memory remember x"}),
            serde_json::json!({"type": "user", "text": "/mem list"}),
            serde_json::json!({"type": "user", "text": ""}),
            serde_json::json!({"type": "user"}),
        ];
        assert_eq!(user_texts(&messages), vec!["hello".to_owned()]);
    }

    #[test]
    fn imperative_rule_proposes_preference() {
        let drafts = propose("Always write tests first.");
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].rule, "imperative-v1");
        assert_eq!(drafts[0].kind, Kind::Preference);
        assert!(!drafts[0].needs_review);
    }

    #[test]
    fn preference_rule_matches_first_person() {
        let drafts = propose("I prefer tabs over spaces for indentation.");
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].rule, "preference-v1");
    }

    #[test]
    fn constraint_scopes_project_when_rooted() {
        let view = view();
        let (drafts, _) = generate(
            &["Services must expose health checks.".to_owned()],
            "ses_1",
            true,
            &view,
        );
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].rule, "constraint-v1");
        assert_eq!(drafts[0].scope, Scope::Project);
        let (drafts, _) = generate(
            &["Services must expose health checks.".to_owned()],
            "ses_1",
            false,
            &view,
        );
        assert_eq!(drafts[0].scope, Scope::User);
    }

    #[test]
    fn careful_classes_flagged_not_dropped() {
        let drafts = propose("We are migrating to Postgres next quarter.");
        assert_eq!(drafts.len(), 1);
        assert!(drafts[0].needs_review);
        let drafts = propose("Priya owns the auth service.");
        assert!(drafts[0].needs_review);
        let drafts = propose("Deploys usually happen on Fridays.");
        assert!(drafts[0].needs_review);
    }

    #[test]
    fn negative_filters_drop() {
        for text in [
            "What version are we on?",
            "```rust\nlet x = 1;\n```",
            "diff --git a/f b/f",
            "@@ -1 +1 @@",
            "Error: connection failed with code 3",
            "See https://example.com/docs for details",
            "Deploy today",
            "Restart pod 3 in staging",
            "ok",
            "Working on it",
        ] {
            let (drafts, stats) = {
                let view = view();
                generate(&[text.to_owned()], "ses_1", false, &view)
            };
            assert!(drafts.is_empty(), "should drop {text:?}");
            assert!(stats.dropped_filter > 0);
        }
    }

    #[test]
    fn secrets_never_proposed() {
        // Split literals: the checkout must never contain contiguous
        // secret-shaped strings (push protection).
        let key = ["sk-", "abcdefghijklmnopqrstuvwxyz"].concat();
        let (drafts, stats) = {
            let view = view();
            generate(
                &[format!("Remember my key {key} for later.")],
                "ses_1",
                false,
                &view,
            )
        };
        assert!(drafts.is_empty());
        assert_eq!(stats.dropped_secret, 1);
    }

    #[test]
    fn assistant_claims_cannot_reach_generator() {
        // The generator only accepts pre-filtered user texts; model text
        // has no input path (structural, 8-R2). user_texts is the gate.
        let messages =
            vec![serde_json::json!({"type": "assistant", "text": "Always write tests first."})];
        assert!(user_texts(&messages).is_empty());
    }

    #[test]
    fn batch_and_store_dedup() {
        let view = view();
        let (drafts, stats) = generate(
            &["Always write tests first. Always write tests first.".to_owned()],
            "ses_1",
            false,
            &view,
        );
        assert_eq!(drafts.len(), 1);
        assert_eq!(stats.dropped_duplicate, 1);
    }

    #[test]
    fn tombstone_blocks_resurrection() {
        let hit: &dyn Fn(Kind, Scope, &str) -> bool = &|_, _, _| true;
        let view = StoreView {
            active: &[],
            tombstone_hit: hit,
        };
        let (drafts, stats) = generate(
            &["Always write tests first.".to_owned()],
            "ses_1",
            false,
            &view,
        );
        assert!(drafts.is_empty());
        assert_eq!(stats.dropped_tombstone, 1);
    }

    #[test]
    fn generation_is_deterministic() {
        let texts = vec!["Always write tests first. I prefer tabs.".to_owned()];
        let view_a = view();
        let view_b = view();
        let (first, _) = generate(&texts, "ses_1", false, &view_a);
        let (second, _) = generate(&texts, "ses_1", false, &view_b);
        assert_eq!(first, second);
    }

    #[test]
    fn sentence_splitting() {
        let sentences =
            split_sentences("Always write tests first. I prefer tabs.\nNever skip reviews.");
        assert_eq!(sentences.len(), 3);
    }

    /// Precision evaluation corpus (gate §10: precision ≥ 0.80, zero
    /// secrets, zero poison, deterministic). Each case states the
    /// expected disposition; the corpus is committed so the number is
    /// reproducible, not tuned per-run.
    #[test]
    fn precision_on_evaluation_corpus() {
        // (sentence, expect_propose)
        let corpus: Vec<(String, bool)> = vec![
            ("Always write tests first.".into(), true),
            ("I prefer tabs over spaces.".into(), true),
            ("Never deploy on Fridays.".into(), true),
            ("Services must expose health checks.".into(), true),
            ("We use Nix for all deploys.".into(), true),
            ("I dislike meetings without agendas.".into(), true),
            ("Remember the staging database resets nightly.".into(), true),
            ("Avoid force-pushing to main.".into(), true),
            ("API keys must rotate every 90 days.".into(), true),
            ("Migrating to Postgres next quarter.".into(), true),
            ("Priya owns the auth service.".into(), true),
            ("Deploys usually happen on Fridays.".into(), true),
            ("I like the new dashboard.".into(), true),
            ("Use the staging cluster for load tests.".into(), true),
            ("Only admins can merge to main.".into(), true),
            ("Never share credentials in chat.".into(), true),
            ("What time is the deploy?".into(), false),
            ("```cargo test```".into(), false),
            ("Error: connection refused".into(), false),
            ("See https://example.com/docs".into(), false),
            ("Restart pod 3 now".into(), false),
            ("ok".into(), false),
            (
                ["Remember my key ", "sk-", "abcdefghijklmnopqrstuvwxyz"].concat(),
                false,
            ),
            ("The build passed.".into(), false),
            ("Can you help me debug this?".into(), false),
            ("Working on the auth refactor".into(), false),
            ("Today the demo went well.".into(), false),
            ("My password is hunter2".into(), false),
            ("TODO: fix the flaky test".into(), false),
            ("The README explains the setup.".into(), false),
        ];
        let view = view();
        let mut tp = 0usize;
        let mut fp = 0usize;
        for (sentence, expected) in &corpus {
            let (drafts, _) = generate(std::slice::from_ref(sentence), "ses_1", false, &view);
            let proposed = !drafts.is_empty();
            if *expected && proposed {
                tp += 1;
            } else if !*expected && proposed {
                fp += 1;
            } else if *expected && !proposed {
                panic!("false negative (recall loss): {sentence:?}");
            }
        }
        let precision = tp as f64 / (tp + fp) as f64;
        assert!(
            precision >= 0.80,
            "precision {precision:.2} (tp={tp} fp={fp}) below gate 0.80"
        );
    }
}
