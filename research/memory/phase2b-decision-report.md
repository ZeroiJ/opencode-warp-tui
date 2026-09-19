# Memory Research Phase 2B — Decision Report & Final Architecture

> Status: 🔬 RESEARCH ONLY — Phase 2B deliverable. **Nothing here is
> implemented.** This is the capstone document: the final Adopt/Build/
> Hybrid decision, the final architecture, the explicit V1 boundary, the
> implementation roadmap, the aggregate test strategy, and the **Phase 2B
> Final Gate** (§9).
>
> Evidence labels: **[VERIFIED]** live probe · **[DOCUMENTED]** official
> docs/specs/source · **[INFERRED]** analysis · **[RECOMMENDATION]** Phase
> 2B decision · **[UNRESOLVED]** open item.
>
> All design detail is in the five companion docs:
> — `phase2b-storage.md` (schema, physical layout, atomicity, tombstones,
>   locations, git policy, SQLite re-evaluation)
> — `phase2b-retrieval.md` (ordering, budget, scope, search rejection)
> — `phase2b-injection.md` (verified boundary, Context Builder, fencing,
>   freezing, command surface)
> — `phase2b-security.md` (threat model, secrets, abuse, privacy)
> — `phase2b-version-resilience.md` (probe, degradation matrix)

---

## 1. How to read this report

Phase 2B's job was to make Phase 2A's candidate *survive technical
scrutiny* — change it where inconsistent or unsafe, and ratify the rest —
then freeze an implementable architecture. Every earlier document
(Phase 1 `architecture.md`, Phase 2A `phase2a-memory-model.md`/
`minimal-architecture.md`) remains on disk, unmodified; **this report and
the decision log supersede** their contested details, with every reversal
documented (storage §4.1, decision-log D15–D24).

---

## 2. Final architecture

```text
┌──────────────────────────────────────────────────────────────┐
│ USER  (types messages; eventually a TUI — unchanged until P9)│
└───────────────┬──────────────────────────────────────────────┘
                │ plain text
                ▼
┌──────────────────────────────────────────────────────────────┐
│ OpenCode Adapter  (Phase 4, unchanged boundaries)            │
│   · /memory … prefix → Memory API            (command routing)│
│   · session start → Context Builder block → injection gate    │
│   · knows active session_id (provenance)                      │
└───────┬──────────────────────────────┬───────────────────────┘
        │ memory commands              │ injection (PUT owt.memory)
        ▼                              ▼
┌───────────────────┐        ┌──────────────────────────┐
│ Memory API        │        │ Capability Gate (probe)  │──┐
│  · validate/refuse│        └──────────────────────────┘  │
│  · keys/status    │                     │                │
└────────┬──────────┘                     ▼                │
         │                       Instruction entries       │
         ▼                       (experimental surface,    │
┌───────────────────┐            version-detected)         │
│ MemoryStore (trait)│             │                       │
│  ✓ file-backed V1 │             ▼                       │
│  (SQLite V2 swap) │      OpenCode session                │
│  ✓ user store     │      Context Epoch baseline          │
│  ✓ project store  │      (position 6 assembly)           │
│  ✓ tombstones     │                                       │
└────────┬──────────┘                                       │
         │ ACTIVE, scoped, ordered                          │
         ▼                                                   │
┌───────────────────┐       ┌──────────────────────────┐    │
│ Context Builder   │──────▶│ fenced, labeled, budgeted │────┘
│  · project→user   │       │ block ≤ 200,000 B         │
│  · pinned→recency │       └──────────────────────────┘
│  · char budget    │
└───────────────────┘
```

Control points (all adapter-side, none inside OpenCode, none in the TUI):

| Point | Owner | Gate |
|---|---|---|
| Ingestion | Memory API | secret refusal + validation (security §4/§6) |
| Persistence | MemoryStore | atomic rewrite + flock (storage §6/§7) |
| Selection/budget | Context Builder | ordering + encode-bound (retrieval §2/§3) |
| Transport | injection gate | capability probe + 262,144 B server limit (version-resilience §2, injection §2) |
| Command routing | adapter prefix | `/memory …` interception (injection §8) |
| Session lifecycle | adapter | block PUT before first prompt, frozen (injection §3/§7) |

**Exactly what this is:** a three-component, file-backed, explicit-user-
command memory engine that rides *behind* the existing Backend boundary
and pushes one fenced, budgeted snapshot into each new OpenCode session
via the instruction-entries surface. Nothing else.

---

## 3. Adopt / Build / Hybrid — final decision: BUILD

Re-scored against **concrete V1 requirements** (Phase 2B brief §27). The
requirements, restated: (R1) explicit-user-only ingestion; (R2) zero
scores/ranking; (R3) whole-corpus-within-budget injection; (R4) zero new
Cargo dependencies; (R5) adapter-side isolation (no in-process OpenCode
presence, no OpenCode config changes); (R6) local-first files; (R7)
survive OpenCode evolution via probing; (R8) privacy: refuse secrets, hard
delete, no content audit; (R9) deterministic, testable.

| Framework | What it actually provides | Solves a V1 problem (R1–R9)? | Dependency burden | Verdict for V1 |
|---|---|---|---|---|
| Hermes-style memory | Agent-integrated memory manager, extraction pipeline, scoring, provider hooks | No (needs extraction + scoring; R1/R2 violated; R5 violated — in-process hooks) | Python agent runtime | **Reject** (reference only: fringe fencing/`<memory-context>` pattern already extracted into our injection §5 — **[VERIFIED]** hermes.md) |
| Hindsight | Memory subsystem: reflector/retriever/prompter, LLM extraction, periodic reflection | No (LLM extraction, per-turn retrieval; R1/R2/R4 violated) | Python + LLM | **Reject** |
| Mem0 | ADD-only store, extraction, graph/vector memory, scoring | No (automatic extraction, embeddings — R1/R2 violated; provider-coupled SaaS) | Python/SDK | **Reject** (its ADD-only no-overwrite discipline endorsed in P2A §10.2) |
| Letta | Agent runtime with persistent memory blocks | No (whole agent framework — R5/R4 violated) | Heavy runtime | **Reject** |
| OpenViking | Rust agent memory, SQLite+FTS5, session management | No (FTS5/ranking engine — R2/R3 violated; AGPL core — licensing constraint documented in Phase 1; reference-only) | Cargo dep | **Reject** (SQLite+FTS5 design ideas absorbed into our V2 migration path, storage §15) |

**Decision: BUILD**, with the record that **no framework solves even two
of the nine V1 requirements**. The V1 requirements are *anti-features* of
every surveyed framework: V1 needs *less* machinery, *no* extraction, *no*
ranking, *no* runtime, *no* dependencies. Adopting any framework would
force R1/R2/R4/R5 violations to get R6.

**Hybrid is the growth path, not V1:** Holographic-style trust feedback,
embeddings sidecars (Option C), and FTS5 arrive V2/V3 *behind the
`MemoryStore` trait* (P2A §17, decision D3), only when extraction makes
them justified. Ratified, dissent recorded: the strongest pro-hybrid
argument — "an existing store saves us file-format work" — fails on R4/R5
and on the fact that our JSONL is already the full V1 storage spec
(storage §5).

---

## 4. Final V1 boundary

### ✅ IN V1 (Phase 5 + 6 scope)

- Store: JSONL user+project stores, rewrite-on-mutation + atomic rename,
  tombstones append-only, flock+mutex (§ storage).
- Schema: the exact `v:1` record (storage §4) — kinds fact|preference,
  scopes user|project, source user (inferred reserved), status
  ACTIVE|SUPERSEDED (DELETED reserved), pinned, created/updated, session
  provenance, optional key/quote/source_ref.
- API commands: `remember`, `update`, `forget` (exact key/id only),
  `list`, `show`; no search.
- Context Builder: ordered, budgeted (12k chars default, ≤200,000 B
  encoded), fenced, labeled block.
- Injection: single `owt.memory` entry at session start, frozen;
  capability probe + degradation tree.
- Secrets: high-confidence refusal, warn-on-label, honest limits.
- Failure isolation: memory ≠ session, everywhere.

### ❌ NOT IN V1 (verified per Phase 2B brief §29)

| Deferred | Why (evidence-backed) |
|---|---|
| Automatic extraction (rules/LLM) | D4: user is sole producer; rules = noise; LLM = model dependency the adapter can't assume |
| Embeddings / vector DB / semantic ranking | Retrieval §7: no V1 consumer; corpus fits the budget; V3 sidecar |
| Search (substring/FTS5/fuzzy) | Retrieval §7: no command consumes it; forget-by-query is deliberately rejected |
| SQLite + FTS5 | Storage §14: no query requirement; file-store inspectability wins; D8 confirmed |
| Entity/graph memory | D9: entities are a retrieval optimization; no retrieval in V1 |
| Conflict engine (CONFLICT) | D5: unreachable without extraction; "allow both" policy covers V1 (retrieval §8) |
| Session memory scope | D2: provenance not scope; history is OpenCode's job |
| Per-turn retrieval / mid-session updates | D12/D18: frozen session-start is the native V2 shape (Context Epoch) |
| Scoring/trust | D3: none has an explainable V1 consumer |
| In-process OpenCode plugins | Isolation: V2 has no system hook anyway **[DOCUMENTED]** (injection §1) |
| TUI memory UI | Phase 9; the API exists behind the Backend boundary from Phase 5 (injection §8) |
| Export/import | Phase 5 CLI `export` only; import V2 (tombstone-guarded) |

---

## 5. Implementation roadmap (incremental; not started)

Each phase below is design only in this document. First implementation
phase is the smallest useful capability: **a store you can read and write
with tests** — no injection yet.

### Phase 5 — Memory Engine Foundation

- **Objective:** working store + Memory API + unit-tested command
  semantics, entirely adapter-side, injection off.
- **Files affected (new, under existing crate layout):**
  `src/backend/memory/{mod,api,store,record,key,tombstone,secret,budget}.rs`
  (+ tests); config keys `memory.*`; no Cargo dep changes.
- **Dependencies:** std only (AGENTS.md).
- **Acceptance:** golden-file store tests (storage §16), secret refusal,
  key/status semantics, atomic-write crash simulation pass, `list/show`
  via adapter routing, `forget` + tombstone.
- **Tests:** storage/security/retrieval unit suites as tabled.
- **Failure condition:** any lost-update or torn-file case in tests, any
  dep added, any TUI/OpenCode modification.

### Phase 6 — OpenCode Memory Integration

- **Objective:** Context Builder + injection gate + version-resilient
  probe wired to the adapter; injection at session start.
- **Files affected:** `src/backend/opencode/{mod,client}.rs` (add memory
  branch), new `src/backend/memory/{context_builder,inject}.rs`.
- **Dependencies:** none; **precondition verification:**
  ([**UNRESOLVED**] §8.1) sandboxed provider call to confirm entry-value
  prompt rendering before shipping.
- **Acceptance:** probe matrix (version-resilience §8); PUT-before-first-
  prompt sequencing test; degradation rows each fixture-tested; golden
  block bytes over live sandboxed server.
- **Failure condition:** rendering verification reverses the value shape
  without a design update; any session message altered by memory failure.

### Phase 8 — Memory Intelligence (later; V2)

Rules extraction with ASK gate (after C3 event-coverage verification),
SQLite+FTS5 swap behind the trait, per-turn retrieval.
**Failure condition:** started before C3 verified or before
Phase 5/6 acceptance.

### Phase 9 — TUI memory UI (later)

`/memory` pane, list/show/forget UI, indicator. Requires Phase 5 API.
**Failure condition:** UI work before the API contract is stable.

Execution rule (AGENTS.md): each phase begins only on explicit
authorization; nothing in this document starts implementation.

---

## 6. Aggregate test strategy (Phase 5/6 — production-ready gate)

Full suites per area are in each companion doc (§16/§10/§11/§10/§8).
Roll-up:

| Suite | Source | Covers |
|---|---|---|
| Unit — schema/key/status | storage §16; retrieval §10 | validation, normalization, transitions, uniqueness |
| Unit — builder/order/budget | retrieval §10; injection §11 | tier order, tie-breaks, char/byte budgets, determinism (golden bytes) |
| Unit — secrets/fencing | security §10 | refusal patterns, warn path, injection-as-data fixtures |
| Storage — atomicity/crash | storage §16 | kill-point simulation, rename visibility, torn-tail, concurrent writers/readers, compaction-free invariant |
| Integration — adapter/injection | injection §11; version-resilience §8 | live sandboxed server round-trips, probe matrix, PUT-before-prompt, degradation rows |
| Security | security §10 | permissions, symlinks, traversal, oversized, DoS fixtures, logs-clean |
| Regression | decision-report §6.1 | **OpenCode works normally with memory disabled** |

### 6.1 The one non-negotiable regression test

```text
memory disabled / unavailable / failing  ⇒  byte-identical OpenCode
session behavior to running with no memory code compiled in.
```

This is the empirical form of `Memory failure ≠ OpenCode failure`
(injection §9, version-resilience §4).

---

## 7. PHASE 2B FINAL GATE

### 1. What is the final architecture?

Three adapter-side components — **Memory API** (explicit commands, secret
refusal), **MemoryStore** (file-backed, user+project, tombstones),
**Context Builder** (budgeted fenced block) — behind the existing Backend
boundary, injecting one frozen entry (`owt.memory`) per session at
session start through the OpenCode instruction-entries surface, with a
capability probe and degradation tree (§2). No TUI change. No OpenCode
change. No in-process plugin.

### 2. Is the final decision ADOPT, BUILD, or HYBRID?

**BUILD.** No framework solves any V1 requirement pair; the requirements
are anti-features of every surveyed system (§3). Hybrid concepts (FTS5,
trust, embeddings) are the V2/V3 growth path behind the same trait.

### 3. Why?

V1 needs *less* than every framework offers; adopting would force
extraction, scoring, runtimes, and dependencies onto a problem that is
"three small local files and one HTTP write" (D1/D8 rationale re-ratified
with fresh f evidence: storage §5, retrieval §7).

### 4. What exactly is V1?

IN V1 / NOT IN V1 boundary in §4. In one sentence: explicit-user-command
memory, stored in two local JSONL stores, injected once per session as a
fenced, budgeted, frozen block, with a hard line between memory failure
and session health.

### 5. What is explicitly deferred?

§4 table: extraction, embeddings/vector/semantic ranking, any search,
SQLite+FTS5, entities/graph, CONFLICT engine, session scope, per-turn
retrieval, scoring, plugins, TUI UI, import (export only). Each with its
decision-log back-reference.

### 6. What storage model is used?

Two JSONL files (user, project) rewritten-on-mutation via temp+fsync+
atomic rename, plus append-only hash-tombstone files, plus lock files;
0600/0700 permissions; project store at `<root>/.owt/`, git-ignored by
default (storage §3/§5/§9–§10).

### 7. What is the exact memory schema?

The `v:1` record in storage §4: `v, id, key?, kind, scope, content,
source, status, pinned, created_at, updated_at, session_id, source_ref?,
quote?`. No tags (D9), no scores (D3), pinned fixed as a field (audit §4.1).

### 8. What is the exact deletion model?

`forget` = exact-key-or-id → resolve ACTIVE → hard-delete the line (atomic
rewrite) → append SHA-256 tombstone of the canonical identity
(kind+scope+normalized content). Explicit re-`remember` is allowed;
V2 extraction/import is blocked by the tombstone. DELETED status reserved.
No content audit trail (storage §11/§13).

### 9. What is the exact retrieval model?

One deterministic ordered read: ACTIVE only → tiers
project-pinned → user-pinned → project-recent → user-recent, each by
`updated_at DESC, created_at DESC, id ASC`; char budget 12k default
(configurable, ≤30k); hard invariant: JSON-encoded value ≤200,000 B
(server limit 262,144 B **[VERIFIED]**); whole-record truncation only;
no search (retrieval §2/§3/§7).

### 10. What is the exact injection boundary?

Instruction-entries surface; live-verified on 2.0.8
(`/api/experimental/session/{id}/instructions/entries`) **[VERIFIED]**,
documented at assembly position 6 **[DOCUMENTED]**; single entry
`owt.memory`, value `{"text": "<fenced block>"}`; written at session
creation before the first prompt; frozen for the session; probe-first
capability gate; fallback = none in V1 (synthetic-message degraded path
documented, unused); never AGENTS.md, never plugins (injection §2/§3/§10).

### 11. What happens when OpenCode changes?

Capability probe per version change; degradation matrix (version-
resilience §3): injection disables cleanly, store commands keep working,
sessions are unaffected; no version-string whitelisting.

### 12. What happens when memory fails?

`Memory failure ≠ OpenCode failure` — one visible warning max, block
absent, session normal; store failures degrade to memory-unavailable;
contention/corruption cases tabled (storage §8, injection §9).

### 13. What are the security guarantees?

File-permission boundary (0600/0700, no-follow/symlink refusal, canonical
paths, no user-path inputs); high-confidence secret refusal + warn-on-label
with documented limits; data-not-instructions fencing with honest
limitations; size guards; no content in logs; no telemetry; hard delete
with tombstone; raw OpenCode history untouched (security §2–§9).

### 14. What unresolved risks remain?

1. **[UNRESOLVED]** Exact prompt rendering of instruction-entry values
   (raw JSON vs. field extraction) — one sandboxed provider probe in
   Phase 6 resolves it; the chosen shape and fence are robust either way.
2. **[INFERRED]** Worktree/rename behavior (path-based project identity) —
   intentional V1 limitation, documented, with an escape hatch
   (copy `.owt/`).
3. **[INFERRED]** Warning-only secret rows could be injected in the wild —
   escalation rule documented (security §4.2).
4. V2 event coverage (Claim C3) still unverified — gates rules extraction
   only, not V1.
5. Instruction-entries surface is `experimental.*` in live 2.0.8 —
   mitigated by probe+degrade; the documented stable alias exists but 404s
   on 2.0.8 **[VERIFIED]**.

### 15. Is the architecture precise enough to authorize implementation?

**Yes.** Every design question Phase 2B was asked has a tabled, evidenced
answer (§1–§14). The remaining items are **verification tasks inside
Phase 5/6**, not blocking design decisions: no Phase 3/5 implementation
may begin without the user's explicit go-ahead, and Phase 6's injection
work is gated on resolving risk 1 in a sandbox.

```text
IMPLEMENTATION AUTHORIZED   (design complete; Phase 5 start still
                             requires explicit user authorization)
```

---

## 8. Unresolved & watch items (detailed)

- **Entry-value rendering (risk 1 above):** we know the value is JSON and
  sits at assembly position 6 **[VERIFIED]/[DOCUMENTED]**; we do not yet
  observe its rendered form without a model call. Phase 6 precondition,
  not a blocker: block format is self-describing.
- **C3 event coverage:** Phase 8 gate, unchanged from P2A §20/Q7.
- **OpenCode docs/source drift:** all DOCUMENTED claims reflect the
  current v2 docs + specs/session.md at review time (2026-09-18); the
  probe design is the corrective mechanism.
- **Warning-only secrets:** revisit condition if ever observed.

---

## 9. Deliverables (this phase)

Created/updated (research only):

| File | Content |
|---|---|
| `phase2b-storage.md` | schema, layout, atomicity, concurrency, tombstones, locations, git policy, SQLite re-eval |
| `phase2b-retrieval.md` | ordering, budget, scope, no-search, staleness, contradictions |
| `phase2b-injection.md` | verified boundary, Context Builder, fencing, freezing, command surface |
| `phase2b-security.md` | threat model, secrets, phishing/abuse, privacy |
| `phase2b-version-resilience.md` | probe, degradation matrix, failure model |
| `phase2b-decision-report.md` | this document |
| `architecture.md` | Phase 2B addendum (supersession note) appended |
| `decision-log.md` | D15–D24 (2B decisions; reversals recorded) |
| `phases.md` | 2B marked COMPLETE; Phase 5 remains PLANNED |

---

## 10. HARD STOP

Phase 2B is complete. Per this phase's rule (Phase 2B brief §37) and
AGENTS.md phase discipline:

- No memory implementation exists or is started by this document.
- No Cargo dependencies are added. No TUI, adapter, or OpenCode files are
  modified. No database, schema, or migration code is written.
- The next action is **the user's explicit authorization to begin
  Phase 5** — nothing in this phase starts it automatically.

```text
ARCHITECTURE STATUS: resolved and specified. Implementation NOT started.
```