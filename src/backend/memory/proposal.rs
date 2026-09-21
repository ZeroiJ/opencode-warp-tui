//! Proposal quarantine queue (Phase 8, 8-R1/intelligence-design §3).
//!
//! Proposals are inert data: they live in `proposals.jsonl` beside the
//! scope store they belong to (same 0600/atomic-rewrite discipline as
//! `memory.jsonl` — private, temp + fsync + rename) and NEVER enter
//! active memory or injection until explicitly confirmed. Discarded
//! proposal hashes persist in `discarded.jsonl` (bounded 500 FIFO) so a
//! discarded candidate is not re-proposed next session-end.
//!
//! Lifecycle: `proposed → confirmed (via remember) | discarded`, plus
//! lazy 30-day expiry enforced on every open (no daemon —
//! intelligence-design §3: "the queue is not memory").

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::record::{parse_rfc3339_utc, Kind, Scope};
use super::tombstone;
use super::MemoryError;

/// Quarantined proposals, one file per scope dir.
pub const PROPOSALS_FILE: &str = "proposals.jsonl";
/// Discarded-proposal hashes, one file per scope dir.
pub const DISCARDED_FILE: &str = "discarded.jsonl";
/// Queue bound (documented FIFO: beyond this, the oldest un-triaged
/// proposal is dropped to admit the new one).
pub const MAX_PROPOSALS: usize = 200;
/// Discarded-hash bound (intelligence-design §3: 500, FIFO).
pub const MAX_DISCARDED: usize = 500;
/// Un-triaged proposal TTL: 30 days, enforced lazily on open.
pub const PROPOSAL_TTL_SECS: u64 = 30 * 24 * 3_600;

/// One quarantined memory proposal. `rule` names the generator rule
/// (`imperative-v1`…); the stored method is always `rule:{rule}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub id: String,
    pub text: String,
    pub kind: Kind,
    pub scope: Scope,
    pub quote: String,
    pub rule: String,
    pub session_id: String,
    pub created_at: String,
    /// CONFIRM-carefully worthiness (goals, relationships, qualified
    /// statements): still proposed, but triage flags it for review.
    pub needs_review: bool,
}

impl Proposal {
    /// The `method` value this proposal writes on confirm.
    pub fn method(&self) -> String {
        format!("rule:{}", self.rule)
    }

    /// Canonical identity shared with tombstones and store dedup
    /// (kind + scope + normalized text).
    pub fn identity_hash(&self) -> String {
        tombstone::canonical_hash(self.kind, self.scope, &self.text)
    }

    pub fn to_json_line(&self) -> String {
        let mut object = serde_json::Map::new();
        object.insert("v".into(), serde_json::Value::from(1));
        object.insert("id".into(), serde_json::Value::from(self.id.clone()));
        object.insert("text".into(), serde_json::Value::from(self.text.clone()));
        object.insert("kind".into(), serde_json::Value::from(self.kind.as_str()));
        object.insert("scope".into(), serde_json::Value::from(self.scope.as_str()));
        object.insert("quote".into(), serde_json::Value::from(self.quote.clone()));
        object.insert("rule".into(), serde_json::Value::from(self.rule.clone()));
        object.insert(
            "session_id".into(),
            serde_json::Value::from(self.session_id.clone()),
        );
        object.insert(
            "created_at".into(),
            serde_json::Value::from(self.created_at.clone()),
        );
        object.insert(
            "needs_review".into(),
            serde_json::Value::from(self.needs_review),
        );
        serde_json::to_string(&serde_json::Value::Object(object))
            .expect("proposal serializes to JSON")
    }

    /// Parse one queue line. Corrupt lines are `Err` (the caller skips +
    /// counts, store discipline — never fatal, never auto-repaired).
    pub fn from_line(line: &str, file_scope: Scope) -> Result<Proposal, String> {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|error| format!("not valid JSON: {error}"))?;
        let object = value
            .as_object()
            .ok_or_else(|| "proposal is not a JSON object".to_owned())?;
        match object.get("v").and_then(serde_json::Value::as_i64) {
            Some(1) => {}
            Some(other) => return Err(format!("unsupported proposal version {other}")),
            None => return Err("missing v".to_owned()),
        }
        let required_str = |field: &str| -> Result<String, String> {
            object
                .get(field)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("missing or non-string {field}"))
        };
        let id = required_str("id")?;
        let text = required_str("text")?;
        if text.trim().is_empty() {
            return Err("empty text".to_owned());
        }
        let kind = object
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .and_then(Kind::parse)
            .ok_or_else(|| "bad kind".to_owned())?;
        let scope = object
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .and_then(Scope::parse)
            .ok_or_else(|| "bad scope".to_owned())?;
        if scope != file_scope {
            return Err("scope does not match file scope".to_owned());
        }
        let rule = required_str("rule")?;
        if rule.trim().is_empty() {
            return Err("empty rule".to_owned());
        }
        let created_at = required_str("created_at")?;
        if parse_rfc3339_utc(&created_at).is_none() {
            return Err("bad created_at".to_owned());
        }
        Ok(Proposal {
            id,
            text,
            kind,
            scope,
            quote: required_str("quote")?,
            rule,
            session_id: required_str("session_id")?,
            created_at,
            needs_review: object
                .get("needs_review")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        })
    }
}

/// Persistent quarantine queue bound to one scope directory. Open fresh
/// per access (load + lazy expiry); call `save` after mutating.
pub struct ProposalQueue {
    dir: PathBuf,
    scope: Scope,
    proposals: Vec<Proposal>,
    /// Discarded identity hashes, oldest-first, capped at MAX_DISCARDED.
    discarded: Vec<String>,
}

impl ProposalQueue {
    /// Open (never creates files — missing files are an empty queue).
    /// Enforces lazy 30-day expiry on load. Corrupt lines are skipped +
    /// counted with a warning, never fatal.
    pub fn open(dir: PathBuf, scope: Scope) -> Result<ProposalQueue, MemoryError> {
        let mut proposals = Vec::new();
        let mut malformed = 0usize;
        let path = dir.join(PROPOSALS_FILE);
        if path.exists() {
            let text = std::fs::read_to_string(&path).map_err(|error| {
                MemoryError::StoreUnavailable(format!("read {}: {error}", path.display()))
            })?;
            for line in text.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                match Proposal::from_line(line, scope) {
                    Ok(proposal) => {
                        if !is_expired(&proposal.created_at) {
                            proposals.push(proposal);
                        }
                    }
                    Err(_) => malformed += 1,
                }
            }
            if malformed > 0 {
                log::warn!(
                    "memory: {malformed} malformed proposal(s) ignored in {}",
                    path.display()
                );
            }
        }
        let mut discarded = Vec::new();
        let discarded_path = dir.join(DISCARDED_FILE);
        if discarded_path.exists() {
            let text = std::fs::read_to_string(&discarded_path).map_err(|error| {
                MemoryError::StoreUnavailable(format!("read {}: {error}", discarded_path.display()))
            })?;
            for line in text.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<serde_json::Value>(line) {
                    Ok(value) => {
                        if let Some(hash) = value.get("hash").and_then(serde_json::Value::as_str) {
                            if hash.starts_with("sha256:") {
                                discarded.push(hash.to_owned());
                                continue;
                            }
                        }
                        malformed += 1;
                    }
                    Err(_) => malformed += 1,
                }
            }
            if malformed > 0 {
                log::warn!(
                    "memory: {malformed} malformed discarded-hash line(s) ignored in {}",
                    discarded_path.display()
                );
            }
        }
        // Cap on load (a foreign writer may have overgrown the file).
        while discarded.len() > MAX_DISCARDED {
            discarded.remove(0);
        }
        Ok(ProposalQueue {
            dir,
            scope,
            proposals,
            discarded,
        })
    }

    pub fn ordered(&self) -> Vec<&Proposal> {
        let mut ordered: Vec<&Proposal> = self.proposals.iter().collect();
        ordered.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        ordered
    }

    pub fn find(&self, id: &str) -> Option<&Proposal> {
        self.proposals.iter().find(|proposal| proposal.id == id)
    }

    pub fn contains_identity(&self, hash: &str) -> bool {
        self.proposals
            .iter()
            .any(|proposal| proposal.identity_hash() == hash)
    }

    /// Admit a proposal. Normalized duplicates already queued are refused
    /// (false, no state change). Beyond MAX_PROPOSALS the oldest is
    /// dropped FIFO to admit the new one (documented bound).
    pub fn push(&mut self, proposal: Proposal) -> bool {
        debug_assert_eq!(proposal.scope, self.scope);
        let hash = proposal.identity_hash();
        if self.contains_identity(&hash) {
            return false;
        }
        self.proposals.push(proposal);
        while self.proposals.len() > MAX_PROPOSALS {
            self.proposals.remove(0);
        }
        true
    }

    /// Remove by exact id (confirm/discard path).
    pub fn remove(&mut self, id: &str) -> Option<Proposal> {
        self.proposals
            .iter()
            .position(|proposal| proposal.id == id)
            .map(|index| self.proposals.remove(index))
    }

    /// Drop every proposal (discard-all path; caller records hashes first).
    pub fn clear(&mut self) {
        self.proposals.clear();
    }

    /// Record a discarded identity hash (bounded 500 FIFO).
    pub fn discard_hash(&mut self, hash: String) {
        if !self.discarded.iter().any(|known| known == &hash) {
            self.discarded.push(hash);
        }
        while self.discarded.len() > MAX_DISCARDED {
            self.discarded.remove(0);
        }
    }

    pub fn is_discarded(&self, hash: &str) -> bool {
        self.discarded.iter().any(|known| known == hash)
    }

    /// Atomic rewrite of both queue files (temp + fsync + rename, 0600).
    pub fn save(&self) -> Result<(), MemoryError> {
        let proposals: Vec<String> = self.proposals.iter().map(Proposal::to_json_line).collect();
        atomic_write(&self.dir, PROPOSALS_FILE, &proposals.join("\n"), true)?;
        let discarded: Vec<String> = self
            .discarded
            .iter()
            .map(|hash| format!(r#"{{"v":1,"hash":"{hash}"}}"#))
            .collect();
        atomic_write(&self.dir, DISCARDED_FILE, &discarded.join("\n"), true)?;
        Ok(())
    }
}

/// True when `created_at` is more than PROPOSAL_TTL_SECS in the past.
/// Unparseable (shouldn't happen — validated on load) and future stamps
/// never expire.
fn is_expired(created_at: &str) -> bool {
    let Some((secs, _)) = parse_rfc3339_utc(created_at) else {
        return false;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    now.saturating_sub(secs) > PROPOSAL_TTL_SECS as i64
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> MemoryError {
    MemoryError::StoreUnavailable(format!("{action} {}: {error}", path.display()))
}

/// Atomic file rewrite mirroring the store discipline (temp + fsync +
/// rename + directory fsync, 0600, no-follow). Empty content still writes
/// the file so reload sees the mutation (e.g. drained queue).
fn atomic_write(
    dir: &Path,
    name: &str,
    content: &str,
    trailing_newline: bool,
) -> Result<(), MemoryError> {
    let temp = dir.join(format!("{name}.tmp.{}", std::process::id()));
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        options.mode(0o600);
        options
            .open(&temp)
            .map_err(|error| io_error("create temp file", &temp, error))?
    };
    #[cfg(not(unix))]
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|error| io_error("create temp file", &temp, error))?;
    if !content.is_empty() {
        file.write_all(content.as_bytes())
            .map_err(|error| io_error("write temp file", &temp, error))?;
        if trailing_newline {
            file.write_all(b"\n")
                .map_err(|error| io_error("write temp file", &temp, error))?;
        }
    }
    file.sync_all()
        .map_err(|error| io_error("fsync temp file", &temp, error))?;
    drop(file);
    std::fs::rename(&temp, dir.join(name))
        .map_err(|error| io_error("rename into place", &temp, error))?;
    if let Ok(dir_file) = std::fs::File::open(dir) {
        let _ = dir_file.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory::record::format_rfc3339_utc;
    use std::time::{Duration, UNIX_EPOCH};

    fn test_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("owt-propq-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn proposal(id: &str, text: &str) -> Proposal {
        Proposal {
            id: id.to_owned(),
            text: text.to_owned(),
            kind: Kind::Preference,
            scope: Scope::User,
            quote: text.to_owned(),
            rule: "imperative-v1".to_owned(),
            session_id: "ses_1".to_owned(),
            created_at: "2026-09-18T10:00:00Z".to_owned(),
            needs_review: false,
        }
    }

    #[test]
    fn empty_queue_opens_without_files() {
        let dir = test_dir("empty");
        let queue = ProposalQueue::open(dir.clone(), Scope::User).unwrap();
        assert!(queue.ordered().is_empty());
        assert!(!dir.join(PROPOSALS_FILE).exists());
    }

    #[test]
    fn push_save_reload_round_trip() {
        let dir = test_dir("roundtrip");
        let mut queue = ProposalQueue::open(dir.clone(), Scope::User).unwrap();
        assert!(queue.push(proposal("owt_1_1_1", "Always write tests first.")));
        // Normalized duplicate refused (whitespace variants hash equal —
        // identity matches the frozen tombstone semantics, which are
        // case-sensitive; case variants are distinct, human-triaged).
        assert!(!queue.push(proposal("owt_1_1_2", "  Always   write   tests   first.  ")));
        queue.save().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(PROPOSALS_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "queue file must be 0600");
        }
        let reloaded = ProposalQueue::open(dir, Scope::User).unwrap();
        assert_eq!(reloaded.ordered().len(), 1);
        assert_eq!(
            reloaded.find("owt_1_1_1").unwrap().text,
            "Always write tests first."
        );
        assert_eq!(
            reloaded.find("owt_1_1_1").unwrap().method(),
            "rule:imperative-v1"
        );
    }

    #[test]
    fn corrupt_lines_skipped_not_fatal() {
        let dir = test_dir("corrupt");
        std::fs::write(
            dir.join(PROPOSALS_FILE),
            "not json\n{\"v\":99,\"id\":\"x\"}\n",
        )
        .unwrap();
        let queue = ProposalQueue::open(dir, Scope::User).unwrap();
        assert!(queue.ordered().is_empty());
    }

    #[test]
    fn remove_and_discard_hashes() {
        let dir = test_dir("discard");
        let mut queue = ProposalQueue::open(dir.clone(), Scope::User).unwrap();
        queue.push(proposal("owt_1_1_1", "Always write tests first."));
        let removed = queue.remove("owt_1_1_1").expect("present");
        queue.discard_hash(removed.identity_hash());
        assert!(queue.ordered().is_empty());
        queue.save().unwrap();
        let reloaded = ProposalQueue::open(dir, Scope::User).unwrap();
        assert!(reloaded.is_discarded(&tombstone::canonical_hash(
            Kind::Preference,
            Scope::User,
            "Always write tests first."
        )));
        assert!(reloaded.find("owt_1_1_1").is_none());
    }

    #[test]
    fn discarded_bounded_fifo() {
        let dir = test_dir("fifo");
        let mut queue = ProposalQueue::open(dir, Scope::User).unwrap();
        for i in 0..(MAX_DISCARDED + 10) {
            queue.discard_hash(format!("sha256:{i:064}"));
        }
        assert_eq!(queue.discarded.len(), MAX_DISCARDED);
        assert!(!queue.is_discarded(&format!("sha256:{:064}", 0)));
        assert!(queue.is_discarded(&format!("sha256:{:064}", MAX_DISCARDED + 9)));
    }

    #[test]
    fn expired_proposals_drop_on_open() {
        let dir = test_dir("expiry");
        let old = UNIX_EPOCH; // 1970: far beyond the 30-day TTL.
        let mut ancient = proposal("owt_1_1_1", "Ancient preference.");
        ancient.created_at = format_rfc3339_utc(old);
        // 31 days ago: expired. 29 days ago: kept.
        let now = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut stale = proposal("owt_1_1_2", "Stale preference.");
        stale.created_at = format_rfc3339_utc(UNIX_EPOCH + Duration::from_secs(now - 31 * 86_400));
        let mut fresh = proposal("owt_1_1_3", "Fresh preference.");
        fresh.created_at = format_rfc3339_utc(UNIX_EPOCH + Duration::from_secs(now - 29 * 86_400));
        let mut queue = ProposalQueue::open(dir.clone(), Scope::User).unwrap();
        queue.push(ancient);
        queue.push(stale);
        queue.push(fresh);
        queue.save().unwrap();
        let reloaded = ProposalQueue::open(dir, Scope::User).unwrap();
        assert!(reloaded.find("owt_1_1_1").is_none());
        assert!(reloaded.find("owt_1_1_2").is_none());
        assert!(reloaded.find("owt_1_1_3").is_some());
    }

    #[test]
    fn ordered_is_created_then_id() {
        let dir = test_dir("ordered");
        let mut queue = ProposalQueue::open(dir, Scope::User).unwrap();
        let mut second = proposal("owt_1_1_2", "Second.");
        second.created_at = "2026-09-18T11:00:00Z".to_owned();
        let mut first = proposal("owt_1_1_1", "First.");
        first.created_at = "2026-09-18T10:00:00Z".to_owned();
        queue.push(second);
        queue.push(first);
        let ids: Vec<&str> = queue.ordered().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["owt_1_1_1", "owt_1_1_2"]);
    }
}
