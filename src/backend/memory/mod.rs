//! Phase 5 — Memory Engine Foundation.
//!
//! A small, local, explicit, deterministic, durable memory store that lives
//! behind the [`Backend`](super::Backend) boundary. This module implements
//! the frozen Phase 2B architecture (`research/memory/phase2b-*.md`,
//! `decision-log.md` D15–D25):
//!
//! ```text
//! Memory API  →  MemoryStore trait  →  JSONL persistence
//! ```
//!
//! - `v:1` records (kinds `fact|preference`, scopes `user|project`,
//!   ACTIVE/SUPERSEDED, pinned, RFC3339 UTC timestamps, session provenance)
//! - user store at `$XDG_DATA_HOME/owt/`, project store at `<root>/.owt/`
//!   (both private: 0700 dirs / 0600 files, no-follow)
//! - rewrite-on-mutation `memory.jsonl` (temp + fsync + atomic rename),
//!   append-only `tombstones.jsonl` (SHA-256 of the canonical identity),
//!   in-process mutex + `flock`-style cross-process lock
//! - deliberate secrets refusal (high-confidence set), control-character
//!   rejection, size guards (10 MiB store, 4096-char content)
//! - `/memory` / `/mem` command routing is owned by the OpenCode adapter
//!   ([`command`]); nothing here talks to OpenCode and **nothing injects
//!   memory into any session** — that is Phase 6.
//!
//! No new Cargo dependencies: everything is std + `serde_json` (already a
//! project dependency).

pub mod api;
/// Phase 6 consumes these pure budget helpers (session-start injection,
/// `research/memory/phase2b-injection.md`). Deliberately unwired in Phase 5
/// — the phase boundary forbids touching any session.
#[allow(dead_code)]
pub mod budget;
pub mod command;
/// Phase 8 rule-based proposal generator (deterministic, user-messages
/// only — 8-R1–R3). Pure history → drafts; quarantine lives in
/// `proposal`.
pub mod extract;
pub mod key;
/// Phase 8 deterministic lexical scorer (ordering only — 8-R7).
/// Dormant until query terms exist; empty terms reproduce base order.
pub mod lexical;
/// Phase 8 proposal quarantine queue (persistent, atomic, bounded).
pub mod proposal;
pub mod record;
pub mod secret;
pub mod store;
pub mod tombstone;
/// Phase 8 triage orchestration over `MemoryApi` (suggest/confirm/
/// discard + session-end refresh). Shared by both backends.
pub mod triage;

pub use api::MemoryApi;

use std::fmt;

/// Controlled memory errors. Kept off the OpenCode session path: a memory
/// failure is reported in-band and the session continues normally.
/// Messages never include memory *content* (ids/keys/reasons only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryError {
    /// The submitted memory violates the schema (bad key/kind/scope/
    /// status/control characters/empty content …). Carries a reason.
    InvalidMemory(String),
    /// High-confidence secret pattern detected; the command is refused and
    /// the store is left byte-identical. NO STATE CHANGE.
    SecretRefused,
    /// Store-level failure (I/O, locked, missing dir, corrupt utf-8 …).
    StoreUnavailable(String),
    /// Permission denied while opening/creating the store.
    PermissionDenied,
    /// Unsupported scope value / unavailable scope target.
    InvalidScope(String),
    /// Key violates `^[a-z0-9][a-z0-9._-]{0,63}$`.
    InvalidKey(String),
    /// No record matches the handle.
    NotFound(String),
    /// Ambiguous resolution (e.g. the same key is ACTIVE in both scopes).
    Conflict(String),
    /// Input exceeds a size cap (content/quote/key/store).
    OversizedInput(String),
    /// The store file exceeds the 10 MiB load guard.
    StoreTooLarge(u64),
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryError::InvalidMemory(reason) => write!(f, "invalid memory: {reason}"),
            MemoryError::SecretRefused => write!(
                f,
                "looks like a secret — not stored (use environment secrets instead)"
            ),
            MemoryError::StoreUnavailable(reason) => {
                write!(f, "memory store unavailable: {reason}")
            }
            MemoryError::PermissionDenied => write!(f, "permission denied accessing memory store"),
            MemoryError::InvalidScope(scope) => write!(f, "invalid memory scope {scope:?}"),
            MemoryError::InvalidKey(key) => write!(
                f,
                "invalid memory key {key:?} (need ^[a-z0-9][a-z0-9._-]{{0,63}}$)"
            ),
            MemoryError::NotFound(handle) => write!(f, "no memory matching {handle:?}"),
            MemoryError::Conflict(detail) => write!(f, "ambiguous memory handle: {detail}"),
            MemoryError::OversizedInput(what) => write!(f, "memory input too large: {what}"),
            MemoryError::StoreTooLarge(bytes) => {
                write!(
                    f,
                    "memory store too large ({bytes} bytes; refusing to load)"
                )
            }
        }
    }
}

impl std::error::Error for MemoryError {}
