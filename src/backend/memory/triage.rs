//! Proposal triage orchestration (Phase 8, 8-R1/8-R12,
//! intelligence-design §3/§7). Thin logic over `MemoryApi`: session-end
//! generation refresh, suggest/confirm/discard with in-band text replies.
//! Both backends (OpenCode adapter, mock) call these — history sourcing
//! stays backend-specific, everything else is shared.
//!
//! Invariants: proposals never enter `memory.jsonl` except through
//! `MemoryApi::remember` on explicit confirm; discard leaves a bounded
//! hash (never content); failures are controlled errors, never panics.

use super::api::{ListFilter, MemoryApi, NewMemoryArgs, ScopeTarget};
use super::extract::{generate, DraftProposal, GenStats, StoreView};
use super::proposal::{Proposal, ProposalQueue};
use super::record::{Kind, Scope};
use super::secret::{scan, SecretVerdict};
use super::{key, MemoryError};

/// Locate a proposal by 1-based suggest index or exact id.
#[derive(Clone, Debug)]
pub enum Target {
    Index(usize),
    Id(String),
}

/// Refresh proposals for a session end (or suggest-time refresh):
/// generate from user texts, admit new drafts to their scope queues.
/// Silent by design — returns counts for logs only. Any failure leaves
/// existing queue state untouched and never affects the session.
pub fn refresh_session_proposals(
    api: &MemoryApi,
    session_id: &str,
    user_texts: &[String],
    project_rooted: bool,
) -> Result<GenStats, MemoryError> {
    let corpus = api.list(&ListFilter::default()).unwrap_or_default();
    let view = StoreView {
        active: &corpus,
        tombstone_hit: &|kind, scope, content| {
            api.tombstone_hit(kind, scope, content).unwrap_or(false)
        },
    };
    let (drafts, stats) = generate(user_texts, session_id, project_rooted, &view);
    if drafts.is_empty() {
        return Ok(stats);
    }
    admit_drafts(api, &drafts)?;
    Ok(stats)
}

/// Admit drafts to their scope queues (dedup vs queue + discarded set).
fn admit_drafts(api: &MemoryApi, drafts: &[DraftProposal]) -> Result<(), MemoryError> {
    use super::record::format_rfc3339_utc;
    let now = format_rfc3339_utc(std::time::SystemTime::now());
    for scope in [Scope::User, Scope::Project] {
        let scoped: Vec<&DraftProposal> = drafts.iter().filter(|d| d.scope == scope).collect();
        if scoped.is_empty() {
            continue;
        }
        let mut queue = api.proposal_queue(scope)?;
        let mut dirty = false;
        for draft in scoped {
            let proposal = Proposal {
                id: key::generate_id(),
                text: draft.text.clone(),
                kind: draft.kind,
                scope: draft.scope,
                quote: draft.quote.clone(),
                rule: draft.rule.clone(),
                session_id: draft.session_id.clone(),
                created_at: now.clone(),
                needs_review: draft.needs_review,
            };
            let hash = proposal.identity_hash();
            if queue.is_discarded(&hash) || !queue.push(proposal) {
                continue;
            }
            dirty = true;
        }
        if dirty {
            queue.save()?;
        }
    }
    Ok(())
}

/// Merged suggest order across scopes: `created_at` ASC, `id` ASC
/// (deterministic; indices are positional at command time).
fn merged_ordered(user: &ProposalQueue, project: Option<&ProposalQueue>) -> Vec<(Scope, String)> {
    let mut entries: Vec<(String, String, Scope)> = Vec::new();
    for proposal in user.ordered() {
        entries.push((
            proposal.created_at.clone(),
            proposal.id.clone(),
            Scope::User,
        ));
    }
    if let Some(queue) = project {
        for proposal in queue.ordered() {
            entries.push((
                proposal.created_at.clone(),
                proposal.id.clone(),
                Scope::Project,
            ));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    entries
        .into_iter()
        .map(|(_, id, scope)| (scope, id))
        .collect()
}

fn open_queues(api: &MemoryApi) -> Result<(ProposalQueue, Option<ProposalQueue>), MemoryError> {
    let user = api.proposal_queue(Scope::User)?;
    let project = if api.has_project() {
        Some(api.proposal_queue(Scope::Project)?)
    } else {
        None
    };
    Ok((user, project))
}

/// Suggest reply: quarantined proposals in deterministic order, clearly
/// labeled as PROPOSALS (never memories).
pub fn suggest_text(api: &mut MemoryApi) -> Result<String, MemoryError> {
    let (user, project) = open_queues(api)?;
    let order = merged_ordered(&user, project.as_ref());
    if order.is_empty() {
        return Ok("no pending proposals".to_owned());
    }
    let mut lines = vec![format!(
        "proposals ({} quarantined — not memories until confirmed):",
        order.len()
    )];
    for (index, (scope, id)) in order.iter().enumerate() {
        let queue = match scope {
            Scope::User => &user,
            Scope::Project => project.as_ref().expect("project queue open"),
        };
        let Some(proposal) = queue.find(id) else {
            continue;
        };
        let flag = if proposal.needs_review {
            " [needs-review]"
        } else {
            ""
        };
        lines.push(format!(
            "{}. [{}|{}]{} {} (id {})",
            index + 1,
            proposal.scope.as_str(),
            proposal.kind.as_str(),
            flag,
            truncate(&proposal.text, 160),
            proposal.id
        ));
        lines.push(format!(
            "   rule {} · {} · {:?}",
            proposal.rule,
            proposal.session_id,
            truncate(&proposal.quote, 80)
        ));
    }
    Ok(lines.join("\n"))
}

fn locate(
    user: &ProposalQueue,
    project: Option<&ProposalQueue>,
    target: &Target,
) -> Result<(Scope, String), MemoryError> {
    let order = merged_ordered(user, project);
    match target {
        Target::Index(n) => {
            if *n == 0 || *n > order.len() {
                return Err(MemoryError::NotFound(format!(
                    "no proposal #{n} ({} pending)",
                    order.len()
                )));
            }
            Ok(order[n - 1].clone())
        }
        Target::Id(id) => order
            .into_iter()
            .find(|(_, known)| known == id)
            .ok_or_else(|| MemoryError::NotFound(format!("no proposal id {id:?}"))),
    }
}

/// Confirm a proposal: re-screen security, re-check tombstones, validate
/// scope/kind, write through `MemoryApi::remember` (existing semantics),
/// remove from the queue. Returns the in-band reply.
pub fn confirm_proposal(
    api: &mut MemoryApi,
    target: &Target,
    scope_override: Option<Scope>,
    kind_override: Option<Kind>,
) -> Result<String, MemoryError> {
    let (user, project) = open_queues(api)?;
    let (scope, id) = locate(&user, project.as_ref(), target)?;
    let queue = match scope {
        Scope::User => &user,
        Scope::Project => project.as_ref().expect("project queue open"),
    };
    let Some(proposal) = queue.find(&id).cloned() else {
        return Err(MemoryError::NotFound(format!("no proposal id {id:?}")));
    };
    // Re-run security at confirm time (never trust queue contents).
    match scan(&proposal.text) {
        SecretVerdict::Refused | SecretVerdict::Warn(_) => {
            remove_and_save(api, scope, &id)?;
            return Err(MemoryError::SecretRefused);
        }
        SecretVerdict::None => {}
    }
    // Re-check tombstones (a forget may have landed after proposing).
    if api.tombstone_hit(proposal.kind, proposal.scope, &proposal.text)? {
        remove_and_save(api, scope, &id)?;
        return Ok("not stored: content was forgotten after proposing".to_owned());
    }
    let scope = scope_override.unwrap_or(proposal.scope);
    if scope == Scope::Project && !api.has_project() {
        return Err(MemoryError::InvalidScope(
            "no project scope (no project root)".into(),
        ));
    }
    let kind = kind_override.unwrap_or(proposal.kind);
    // Normalized dup vs ACTIVE: confirm becomes already-stored notice
    // (proposals carry no keys, so the same-key update path cannot
    // trigger — no duplicate ACTIVE row is ever written).
    let active = api.active_in(scope)?;
    if active.iter().any(|record| {
        record.kind == kind
            && super::record::normalize_content(&record.content)
                == super::record::normalize_content(&proposal.text)
    }) {
        remove_and_save(api, scope, &id)?;
        return Ok("already stored — proposal dropped, no duplicate written".to_owned());
    }
    let args = NewMemoryArgs {
        key: None,
        kind,
        content: proposal.text.clone(),
        pinned: false,
        scope: match scope {
            Scope::User => ScopeTarget::User,
            Scope::Project => ScopeTarget::Project,
        },
        method: Some(crate::backend::memory::record::Method::Rule(
            proposal.rule.clone(),
        )),
        quote: Some(proposal.quote.clone()),
        session_id: Some(proposal.session_id.clone()),
    };
    let result = api.remember(&proposal.session_id, &args)?;
    remove_and_save(api, scope, &id)?;
    // Same-scope rows for conflict context (§4: human sees potential
    // conflicts at triage time), most lexically relevant to the
    // confirmed text first (ordering-only scorer use).
    let mut context_rows = api.active_in(scope)?;
    context_rows.retain(|record| record.id != result.outcome.created.id);
    super::lexical::order_records(&mut context_rows, &super::lexical::tokenize(&proposal.text));
    let context: Vec<String> = context_rows
        .into_iter()
        .take(3)
        .map(|record| truncate(&record.content, 60))
        .collect();
    let mut reply = format!(
        "confirmed ({}) id {} kind={} method={}",
        scope.as_str(),
        result.outcome.created.id,
        kind.as_str(),
        proposal.method()
    );
    if !context.is_empty() {
        reply.push_str("\nsame-scope context: ");
        reply.push_str(&context.join(" · "));
    }
    if let Some(warning) = &result.warning {
        reply.push_str("\nnote: ");
        reply.push_str(warning);
    }
    Ok(reply)
}

fn remove_and_save(api: &mut MemoryApi, scope: Scope, id: &str) -> Result<(), MemoryError> {
    let mut queue = api.proposal_queue(scope)?;
    queue.remove(id);
    queue.save()
}

/// Discard by index/id, or `None` (= all, requires `confirm_all`).
/// Discarding records the identity hash (bounded 500 FIFO) so the
/// candidate is not immediately re-proposed.
pub fn discard_proposal(
    api: &mut MemoryApi,
    target: Option<&Target>,
    confirm_all: bool,
) -> Result<String, MemoryError> {
    let Some(target) = target else {
        let (user, project) = open_queues(api)?;
        let order = merged_ordered(&user, project.as_ref());
        if !confirm_all {
            if order.is_empty() {
                return Ok("no pending proposals".to_owned());
            }
            return Ok(format!(
                "This discards {} proposal(s). Repeat as `/memory discard all --confirm` to proceed.",
                order.len()
            ));
        }
        let mut count = 0usize;
        for scope in [Scope::User, Scope::Project] {
            if scope == Scope::Project && !api.has_project() {
                continue;
            }
            let mut queue = api.proposal_queue(scope)?;
            let hashes: Vec<String> = queue
                .ordered()
                .iter()
                .map(|proposal| proposal.identity_hash())
                .collect();
            for hash in hashes {
                queue.discard_hash(hash);
                count += 1;
            }
            queue.clear();
            queue.save()?;
        }
        return Ok(format!("discarded {count} proposal(s)"));
    };
    let (user, project) = open_queues(api)?;
    let (scope, id) = locate(&user, project.as_ref(), target)?;
    let mut queue = api.proposal_queue(scope)?;
    let Some(proposal) = queue.remove(&id) else {
        return Err(MemoryError::NotFound(format!("no proposal id {id:?}")));
    };
    queue.discard_hash(proposal.identity_hash());
    queue.save()?;
    Ok(format!("discarded proposal id {id}"))
}

fn truncate(text: &str, max: usize) -> String {
    let count = text.chars().count();
    let mut out: String = text.chars().take(max).collect();
    if count > max {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory::api::{ListFilter, ScopeTarget};
    use crate::backend::memory::record::{Kind, Scope};

    fn temp_api(name: &str, project: bool) -> MemoryApi {
        let base =
            std::env::temp_dir().join(format!("owt-triage-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user_dir = base.join("user");
        let project_root = if project {
            Some(base.join("root"))
        } else {
            None
        };
        MemoryApi::open(user_dir, project_root)
    }

    fn refresh(api: &mut MemoryApi, texts: &[&str]) -> GenStats {
        refresh_session_proposals(
            api,
            "ses_1",
            &texts.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>(),
            api.has_project(),
        )
        .expect("refresh works")
    }

    #[test]
    fn suggest_lists_quarantined_not_memories() {
        let mut api = temp_api("suggest", false);
        refresh(&mut api, &["Always write tests first."]);
        let text = suggest_text(&mut api).expect("suggest works");
        assert!(text.contains("quarantined"), "got: {text}");
        assert!(text.contains("Always write tests first."));
        assert!(text.contains("imperative-v1"));
        // Nothing entered active memory.
        assert!(api.list(&ListFilter::default()).unwrap().is_empty());
    }

    #[test]
    fn confirm_by_index_stores_with_method() {
        let mut api = temp_api("confirm-idx", false);
        refresh(&mut api, &["Always write tests first."]);
        let reply = confirm_proposal(&mut api, &Target::Index(1), None, None).expect("confirmed");
        assert!(reply.contains("confirmed"), "got: {reply}");
        assert!(reply.contains("method=rule:imperative-v1"), "got: {reply}");
        let listed = api.list(&ListFilter::default()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].method,
            Some(crate::backend::memory::record::Method::Rule(
                "imperative-v1".to_owned()
            ))
        );
        assert_eq!(listed[0].session_id, "ses_1");
        // Queue drained.
        assert_eq!(suggest_text(&mut api).unwrap(), "no pending proposals");
    }

    #[test]
    fn confirm_by_id_with_overrides() {
        let mut api = temp_api("confirm-id", true);
        refresh(&mut api, &["I prefer tabs over spaces."]);
        let text = suggest_text(&mut api).unwrap();
        let id = text
            .lines()
            .find_map(|line| line.split("(id ").nth(1))
            .and_then(|rest| rest.strip_suffix(')'))
            .expect("suggest shows id")
            .to_owned();
        let reply = confirm_proposal(
            &mut api,
            &Target::Id(id),
            Some(Scope::Project),
            Some(Kind::Fact),
        )
        .expect("confirmed");
        assert!(reply.contains("(project)"), "got: {reply}");
        let listed = api.list(&ListFilter::default()).unwrap();
        assert_eq!(listed[0].scope, Scope::Project);
        assert_eq!(listed[0].kind, Kind::Fact);
    }

    #[test]
    fn confirm_unknown_target_errors() {
        let mut api = temp_api("confirm-miss", false);
        let err = confirm_proposal(&mut api, &Target::Index(1), None, None).unwrap_err();
        assert!(matches!(err, MemoryError::NotFound(_)));
        let err =
            confirm_proposal(&mut api, &Target::Id("owt_1_2_3".into()), None, None).unwrap_err();
        assert!(matches!(err, MemoryError::NotFound(_)));
    }

    #[test]
    fn confirm_rejects_project_without_root() {
        let mut api = temp_api("confirm-scope", false);
        refresh(&mut api, &["Always write tests first."]);
        let err =
            confirm_proposal(&mut api, &Target::Index(1), Some(Scope::Project), None).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidScope(_)));
        // Proposal still queued (no state change on failed confirm).
        assert_ne!(suggest_text(&mut api).unwrap(), "no pending proposals");
    }

    #[test]
    fn confirm_duplicate_reports_already_stored() {
        let mut api = temp_api("confirm-dup", false);
        refresh(&mut api, &["Always write tests first."]);
        confirm_proposal(&mut api, &Target::Index(1), None, None).unwrap();
        // Same text proposed again (new session): admitted (store check
        // happens at confirm, not at propose — wait, propose checks the
        // store too, so refresh yields nothing new).
        let stats = refresh(&mut api, &["Always write tests first."]);
        assert_eq!(stats.dropped_duplicate, 1);
        assert_eq!(suggest_text(&mut api).unwrap(), "no pending proposals");
    }

    #[test]
    fn discard_by_index_and_id() {
        let mut api = temp_api("discard", false);
        refresh(&mut api, &["Always write tests first.", "I prefer tabs."]);
        let reply = discard_proposal(&mut api, Some(&Target::Index(1)), false).expect("discarded");
        assert!(reply.contains("discarded"), "got: {reply}");
        // Re-proposing the discarded text does NOT resurface it (the
        // discarded-set check runs at queue admission, so the queue —
        // not the generation count — is the assertion level here).
        refresh(&mut api, &["Always write tests first."]);
        let text = suggest_text(&mut api).unwrap();
        assert!(!text.contains("Always write tests"), "got: {text}");
        assert!(text.contains("I prefer tabs"));
        // The other proposal confirms by id.
        let text = suggest_text(&mut api).unwrap();
        let id = text
            .lines()
            .find_map(|line| line.split("(id ").nth(1))
            .and_then(|rest| rest.strip_suffix(')'))
            .expect("suggest shows id")
            .to_owned();
        discard_proposal(&mut api, Some(&Target::Id(id)), false).unwrap();
        assert_eq!(suggest_text(&mut api).unwrap(), "no pending proposals");
    }

    #[test]
    fn discard_all_needs_literal_confirm() {
        let mut api = temp_api("discard-all", false);
        refresh(&mut api, &["Always write tests first.", "I prefer tabs."]);
        // Bare `all` only explains; nothing is discarded.
        let reply = discard_proposal(&mut api, None, false).expect("explains");
        assert!(reply.contains("--confirm"), "got: {reply}");
        assert_ne!(suggest_text(&mut api).unwrap(), "no pending proposals");
        // Literal token proceeds.
        let reply = discard_proposal(&mut api, None, true).expect("discards");
        assert!(reply.contains("discarded 2"), "got: {reply}");
        assert_eq!(suggest_text(&mut api).unwrap(), "no pending proposals");
    }

    #[test]
    fn forgotten_content_cannot_resurrect() {
        let mut api = temp_api("resurrect", false);
        // Store then forget: tombstone now covers the identity.
        api.remember(
            "ses_1",
            &NewMemoryArgs {
                key: None,
                kind: Kind::Preference,
                content: "Always write tests first.".to_owned(),
                pinned: false,
                scope: ScopeTarget::Default,
                method: None,
                quote: None,
                session_id: None,
            },
        )
        .unwrap();
        api.forget(
            &crate::backend::memory::store::Handle::Key("nope".into()),
            ScopeTarget::Default,
        )
        .unwrap_err();
        // Forget by content: no key — use id lookup via list.
        let id = api.list(&ListFilter::default()).unwrap()[0].id.clone();
        api.forget(
            &crate::backend::memory::store::Handle::Id(id),
            ScopeTarget::Default,
        )
        .unwrap();
        let stats = refresh(&mut api, &["Always write tests first."]);
        assert_eq!(stats.dropped_tombstone, 1);
        assert_eq!(suggest_text(&mut api).unwrap(), "no pending proposals");
    }

    #[test]
    fn secret_rescreened_at_confirm() {
        let mut api = temp_api("rescreen", false);
        refresh(&mut api, &["Always write tests first."]);
        // Simulate a queue entry that turned secret-shaped (e.g. edited
        // file): confirm must refuse and drop it, never store.
        {
            let mut queue = api.proposal_queue(Scope::User).unwrap();
            let mut proposal = queue
                .remove(
                    &queue
                        .ordered()
                        .first()
                        .map(|p| p.id.clone())
                        .expect("one proposal"),
                )
                .expect("present");
            proposal.text = ["my token is ", "abcdef123456"].concat();
            queue.push(proposal);
            queue.save().unwrap();
        }
        let err = confirm_proposal(&mut api, &Target::Index(1), None, None).unwrap_err();
        assert_eq!(err, MemoryError::SecretRefused);
        assert!(api.list(&ListFilter::default()).unwrap().is_empty());
    }

    #[test]
    fn quarantined_proposals_never_inject() {
        // Injection reads ordered_active (ACTIVE store rows only). A queue
        // full of proposals changes nothing there.
        let mut api = temp_api("quarantine", false);
        refresh(&mut api, &["Always write tests first.", "I prefer tabs."]);
        assert!(api.ordered_active().unwrap().is_empty());
        assert_ne!(suggest_text(&mut api).unwrap(), "no pending proposals");
    }
}
