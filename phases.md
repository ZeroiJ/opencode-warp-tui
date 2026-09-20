# Project Phases — authoritative roadmap and phase tracker

> Update this file whenever a phase starts or finishes. Never mark work
> complete without verification. See Maintenance Rules at the bottom.

## Vision

A standalone Rust terminal UI that speaks the visual/interaction language of
Warp's TUI while using OpenCode as the underlying agent/backend.

```
Warp-style TUI
      │
      ▼
Generic Backend trait
      │
      ▼
OpenCodeBackend
      │
      ▼
OpenCode API / SDK / event stream
```

A separate long-term memory subsystem is planned, researched specifically
for OpenCode compatibility:

```
OpenCode
    │
    ▼
OpenCode Adapter
    │
    ├──────────────► Warp-style TUI
    │
    └──────────────► Memory Engine
                          │
                          ▼
                     Retrieval
                          │
                          ▼
                   Context Builder
                          │
                          ▼
                     OpenCode
```

## Phase status

| Phase | Status |
| --- | --- |
| Phase 1 — Warp TUI Research / Extraction | ✅ COMPLETE |
| Phase 2 — Standalone Warp-Style TUI | ✅ COMPLETE |
| Phase 3 — Frontend Hardening | ✅ COMPLETE |
| Phase 4 — OpenCode Adapter Foundation | ✅ COMPLETE |
| Memory Research Phase 1 — OpenCode-Compatible Memory Architecture | ✅ COMPLETE |
| Memory Research Phase 2A — Memory Model & Minimality Review | ✅ COMPLETE |
| Memory Research Phase 2B — Storage, Schema & Injection Architecture | ✅ COMPLETE |
| Phase 5 — Memory Engine Foundation | ✅ COMPLETE |
| Phase 6A — OpenCode Memory Integration Reconnaissance | ✅ COMPLETE |
| Phase 6B — OpenCode Memory Integration Implementation | ⏳ PLANNED |
| Phase 7A — Research & Architecture Reconnaissance | ✅ COMPLETE |
| Phase 7 — Full Agent Interaction | ⏳ PLANNED |
| Phase 8 — Memory Intelligence | ⏳ PLANNED |
| Phase 9 — TUI Memory UX | ⏳ PLANNED |
| Phase 10 — Production Hardening | ⏳ PLANNED |

## Phase 1 — Warp TUI Research / Extraction — ✅ COMPLETE

* Objective: investigate the Warp repository and isolate its CLI/agent TUI.
* Investigated the repository structure and identified the TUI architecture
  (`crates/warp_tui`), the rendering foundation (MIT `warpui_core` cell-grid
  element library), and the backend coupling surface (`warp::tui_export`).
* Mapped components, state, rendering functions, events, and dependencies
  (`research/warp-tui-map.md`, `research/dependency-map.md`).
* Investigated licensing: split MIT (`warpui`/`warpui_core`) vs AGPL-3.0-only
  (everything else, including `warp_tui`); documented in
  `research/licensing.md`.
* Created the isolated project with 7 presentation-only snapshots under
  `warp-tui/` (pristine, frozen). Only selected components were extracted;
  substantial Warp-specific coupling was recorded, not copied.
* `~/warp` was not modified.
* Next phase: Phase 2.

## Phase 2 — Standalone Warp-Style TUI — ✅ COMPLETE

* Objective: compilable standalone TUI foundation on local abstractions.
* Produced the `opencode-warp-tui` crate (binary `owt`) rendering genuine
  `warpui_core` elements through `TuiRuntime`.
* Created the local `Backend` trait + scripted `MockBackend`; session UI,
  transcript renderer, tool/agent views, permission/question gates, menus,
  shell mode, statusline, `>`/`!` prompt, Warp-style layout.
* Theme seam (`src/theme.rs`) replacing Warp's `TuiUiBuilder`.
* Tests + live terminal verification (tmux captures, ANSI inspection).
* OpenCode integration intentionally NOT implemented.
* Known gaps carried forward (recorded in `research/phase2.md`): no
  streaming, single-line prompt, unhosted `tab_bar.rs`, unverified wheel
  sign, no keymap doc.
* Next phase: Phase 3.

## Phase 3 — Frontend Hardening — ✅ COMPLETE

* Objective: polished backend-independent frontend shell.
* Streaming architecture: polled `StreamEvent` FIFO + repaint-driven pump
  (spawned timers don't run under `TuiRuntime` — verified in Warp source).
* `StreamEvent::{Chunk, Push, UpdateTool, ShellLine, WorkLabel}` shared by
  all backends (`src/backend/stream.rs`).
* Real Warp `TuiTabBarView` hosted as an App child view; `ctrl-p/n/o`.
* Multiline prompt (`ctrl-j`/shift-enter, cross-line editing, sanitized
  multiline paste); live `StatuslineElement`; explicit pin scroll model
  (replaced the broken `usize::MAX` sentinel); corrected wheel sign against
  the runtime's `ScrollUp → (0, 1)` convention; transcript control-byte
  sanitizing and mid-word wrap.
* Backend lifecycle ops (`new_session`, `cancel`, `agent_status`);
  deterministic `/demo a–j` MockBackend scenarios.
* `scripts/pty_probe.py`: direct-pty harness with exact ANSI grid parser
  (18 checks: scroll/wheel/paste/newline/streaming/gates/cancel/tabs/help/
  exit/resizes). 34 unit tests incl. event-routing through a real `App`.
* `research/keymap.md`; `cargo fmt` clean; zero clippy warnings.
* Known limitations (NOT silently solved — see `research/phase3.md`):
  mouse clicks untestable in-sandbox; shift-enter needs Kitty enhancement;
  stream-created tabs sync on next action; narrow statusline truncation;
  approximated dark theme; markdown-lite; no selection/vim/completion.
* Next phase: Phase 4.

## Phase 4 — OpenCode Adapter Foundation — ✅ COMPLETE

* Objective: connect the TUI to OpenCode without making it OpenCode-specific.
* Investigated the real interfaces (see `research/opencode-architecture.md`):
  binary 1.18.31 + live managed service 2.0.1; HTTP + SSE; session/message
  models; dual event vocabularies (`session.*` live, `session.next.*` in
  source — both mapped); permission (once/always/reject) and question
  models; tool/shell lifecycle; per-session interrupt; Basic
  `opencode:<password>` auth; `service.json` discovery.
* `OpenCodeBackend` (`src/backend/opencode/{mod,config,client,events,
  mapper}.rs`) implements the same generic trait: endpoint resolve
  (env → discovery → private spawn) → verify → load → SSE thread; snapshots
  from memory; blocking single-roundtrip REST; worker channel drained by the
  existing pump. TUI cannot distinguish it from the mock.
* Minimal justified trait evolution: string session ids, `ToolCall.output`,
  `ThinkChunk`, session summaries/blocks snapshots.
* Event translation is pure + tested; question answering honestly unwired
  (no route observed — records a Notice instead of pretending).
* Verified live: real sessions/tabs/history/status, submit→streaming
  text+reasoning+completion, shell tool end-to-end, gates render; read-only
  integration test; ephemeral sessions and credential copies cleaned up.
* 54 unit tests green; 18/18 mock harness checks still green; MockBackend
  untouched in behavior.
* The TUI remains independent of OpenCode types: all OpenCode logic lives
  behind the adapter boundary.
* Next phase: Memory Research Phase 1.

## Current phase — 🔬 MEMORY RESEARCH PHASE 2B — ✅ COMPLETE

Title: **Storage, Schema & Injection Architecture**. Research/design only —
the final architecture decision package. Nothing implemented.

Objective: resolve the Phase 2A open questions (§20), verify assumptions
against the running OpenCode 2.0.8 server + current docs/specs, produce
the final Adopt/Build/Hybrid decision, and freeze the V1 design precisely
enough that Phase 5 can implement without making architecture decisions.

### Phase 2B outcome — the final architecture

1. **Final decision: BUILD** (three components: Memory API, MemoryStore,
   Context Builder). ADOPT/HYBRID rejected for V1 (`phase2b-decision-report.md` §3).
2. **Storage**: JSONL store per scope, **rewrite-on-mutation** (temp +
   fsync + atomic rename) resolving the Phase 2A append-only-vs-delete
   contradiction; append-only hash tombstones; flock + in-process mutex;
   user store `$XDG_DATA_HOME/owt/`, project store `<root>/.owt/`
   git-ignored by default (D15/D20/D21).
3. **Exact `v:1` schema** fixed (kinds fact|preference, scopes user|
   project, status ACTIVE|SUPERSEDED, pinned, provenance, optional key/
   quote/source_ref; no tags, no scores) (D16).
4. **Injection boundary verified live**: instruction entries
   (`/api/experimental/session/{id}/instructions/entries`) — PUT/GET/DELETE
   204s; **262,144-byte value limit measured (413)**; assembly position 6
   documented; single `owt.memory` entry written at session start, frozen.
   V2 Context Epoch documented model validates session-start freezing
   (D17/D18).
5. **Command surface**: adapter-side `/memory` prefix routing — memory
   works without any TUI change (Phase 9 delivers the UI) (D19).
6. **Retrieval**: deterministic tier order (project-pinned → user-pinned →
   project-recent → user-recent); 12k-char default budget with hard
   ≤200,000-byte encoded invariant; **no search in V1** (D24).
7. **Version resilience**: capability probe per server version + full
   degradation matrix; memory failure ≠ OpenCode failure everywhere (DR §7).
8. **Security**: 0600/0700 + no-follow; small high-confidence secret-
   refusal set + warn-on-label; injection-as-data fencing with honest
   limits; privacy contract reaffirmed (D23, `phase2b-security.md`).

Deliverables: `phase2b-storage.md`, `phase2b-retrieval.md`,
`phase2b-injection.md`, `phase2b-security.md`, `phase2b-version-resilience.md`,
`phase2b-decision-report.md`; updates to `architecture.md` (addendum),
`decision-log.md` (D15–D25), this file.

### Architecture status

```
IMPLEMENTATION AUTHORIZED (design complete). Implementation NOT started.
```

Phase 2B is research/design only: no memory engine, no store, no Cargo
dependencies, no TUI/adapter/OpenCode modifications. **Phase 5 begins only
on explicit user authorization.**

---

## Memory Research Phase 2A — Memory Model & Minimality Review — ✅ COMPLETE

Title: **Memory Model & Minimality Review**. Research only — no memory
implementation until this research is complete.

Objective: critically review the Phase 1 candidate architecture and
determine the **smallest useful memory model** before freezing storage,
schema, retrieval, or implementation decisions.

### Phase 2A outcome — the minimum memory model

1. **Three components**: Memory API (explicit user commands), MemoryStore
   (file-backed, user + project scopes), Context Builder (fenced, labeled,
   budgeted session-start injection).
2. **Two memory kinds** (`fact`, `preference`); **two scopes** (`user`,
   `project`) — SESSION is provenance, not a scope.
3. **One source label** (`user` vs `inferred`) replaces fact-vs-inference
   plus confidence; **zero stored scores** (no trust/confidence/importance/
   novelty/etc.).
4. **Lifecycle**: ACTIVE / SUPERSEDED / DELETED; never silent overwrite;
   correction = supersession; forget = hard delete + hash tombstone.
5. **Ingestion is explicit user request only** (Model A); rules extraction
   lands in V2, LLM extraction in V3 — both opt-in.
6. **V1 storage is a plain file store** (JSONL, no new Cargo deps) behind
   the `MemoryStore` trait; SQLite + FTS5 is re-evaluated in Phase 2B for
   V2.
7. **Options re-scored**: A (curated in-context) = the correct V1; B
   (adapter-side engine) = the V2 growth path behind the same interfaces;
   C (sidecar) = deferred until semantic/graph features are wanted.

Deliverables: `research/memory/phase2a-memory-model.md`,
`research/memory/minimal-architecture.md`, `research/memory/decision-log.md`
(14 decisions, D1–D14).

### Architecture status

```
Candidate refined, implementation NOT authorized.
```

Phase 2A is research only. No memory engine, no store, no database, no
Cargo dependencies, no TUI/OpenCode modification. V1 (Phase 5) is not
authorized; Phase 2B decides architecture, then Phase 5 implements.

## Future phases

### Memory Research Phase 2B — ✅ COMPLETE (see the current-phase section above)

Storage/schema, retrieval, injection boundary, version resilience,
security, and the final Adopt/Build/Hybrid decision are resolved:
`phase2b-decision-report.md` + five companion docs; decisions D15–D25.

### Phase 5 — Memory Engine Foundation — ✅ COMPLETE

* Objective: the memory engine behind the adapter — Memory API + `MemoryStore`
  trait + JSONL persistence + `/memory` command routing. **Injection stays
  off** (Phase 6); no new Cargo dependencies (std + existing `serde_json`).
* Architecture per frozen Phase 2B: `MemoryApi → MemoryStore → JSONL`, one
  scope per `JsonlStore` instance. Exact `v:1` schema (D16): keys
  `^[a-z0-9][a-z0-9._-]{0,63}$` (lowercase-normalized), content ≤ 4096 chars
  with C0 rejection, RFC3339 UTC `Z` (hand-rolled civil-date conversion,
  tested against known epochs), `session_id` required, ids
  `owt_<unix_millis>_<pid>_<seq>`.
* Storage: user store `$XDG_DATA_HOME/owt/` (fallback `~/.local/share/owt/`),
  project store `<root>/.owt/` (gitignored). Rewrite-on-mutation `memory.jsonl`
  (temp + fsync + rename + dir fsync, 0700 dirs / 0600 files, `OPEN_NOFOLLOW`),
  append-only `tombstones.jsonl` (SHA-256 of the canonical identity — hash
  only, appended **before** the rewrite), in-process mutex + `File::lock`
  + re-read-under-lock, 10 MiB load guard, malformed lines skipped + warned +
  counted (never fatal).
* Segregation: project store failure disables only project commands; user
  store keeps working. Scope resolution per D24/retrieval §7.1: writes default
  to **user** (`--scope=project` opt-in), reads merge both scopes, forget/pin
  cross-scope resolve with Conflict on ambiguity.
* Secrets (D23): hard refusal of high-confidence patterns (`sk-`/`pk-`+20,
  PEM armor, `ghp_`/`github_pat_`+20, `AKIA…`, `xox…`) with **no state change
  on refusal**; proximity (`label: value`) and `user:pass@` warnings surfaced
  in-band but stored.
* Commands routed in `OpenCodeBackend::submit` **before any network call**:
  `/memory` + `/mem`, `\/memory` escape, malformed input never becomes a
  prompt, in-band Assistant/Error replies, memory failure never fatal.
  `remember/update/forget/pin/unpin/list/show/help`.
* Budget helper (`budget.rs`) is a pure, tested module — `select` +
  `render_block` + `encoded_block` per injection §3.1/§4.2 (12 000 default /
  ≤30 000-chars / ≤200 000 encoded bytes; D24 order into the block, golden
  block format locked by test). **Nothing writes it into a session.**
* Verification: `cargo fmt --check` clean; clippy `--all-targets
  --all-features -- -D warnings` zero; **144 tests** (143 pass, 1 ignored fork
  test) incl. two-real-processes flock, concurrent no-lost-update, golden
  block, NIST SHA-256 vectors; 18/18 mock pty harness checks green; MockBackend
  and TUI untouched.
* Known limitations (recorded, not hidden): tombstone read-side API
  (`load_hashes`/`contains`) has no consumer yet (Phase 5 writes only);
  `ordered_active`/budget consumed by Phase 6; no memory UX in the TUI; no
  auto-extraction; OpenCode live integration deliberately untested (adapter
  only described, injection off).
* Next phase: Phase 6 (OpenCode Memory Integration) — **start only on
  explicit user authorization**.

### Phase 6A — OpenCode Memory Integration Reconnaissance — ✅ COMPLETE

* Objective (research/verification/design only — no implementation): trace
  OWT session creation, verify the instruction-entry surface live on
  OpenCode 2.0.8, recheck the 262,144-byte limit, resolve the Phase 2B
  rendering UNRESOLVED item, and freeze the Phase 6B contract.
* Verified live: session create → `data.id`; entries PUT/GET/DELETE 204,
  mutable, session-scoped, key regex 400-enforced; stable alias still 404;
  200 KiB accepted / 300 KiB → 413 `maxBytes:262144`; no transcript
  pollution (messages + context empty post-PUT).
* Rendering resolved by controlled probe (throwaway session, test-only
  content, deleted after): entry arrives as system-prompt context text —
  newlines/quotes/JSON/footer intact, no tool calls. One empty-body
  post-turn entry-GET recorded as watch-only anomaly (V1 never reads back).
* Outcome: no Phase 2B decision revised (D17 open item completed);
  Phase 6B contract = consume `ordered_active` + `budget::{select,
  render_block, encoded_block}`; add 2 thin client wrappers, version-keyed
  self-cleaning probe, hook in the two create paths (never resume), config
  knob `memory.injection = auto|off`.
* Deliverables: `research/memory/phase6a-recon.md`,
  `research/memory/phase6a-decision-log.md` (6A-R1…R10). No `.rs` touched;
  `~/warp` untouched; no `6a-` sessions left on the server.
* Next phase: Phase 6B (implementation) — **start only on explicit user
  authorization**.

### Phase 6B — OpenCode Memory Integration Implementation — ⏳ PLANNED

Context Builder wiring + injection gate + capability probe in the
adapter; single `owt.memory` entry at session start, frozen — per the
Phase 6A contract. **Start only on explicit user authorization.**

### Phase 7A — Research & Architecture Reconnaissance — ✅ COMPLETE

* Research-only: roadmap evidence (phases.md + README agree: Phase 7 =
  Full Agent Interaction, not memory UI — that is Phase 9), Phase 4
  leftover reconciliation, read-only adapter/TUI inspection, live 2.0.8
  OpenAPI surface mapping (113 paths, read-only fetch).
* Findings: `answer_question` unwired (no `/question/` route on 2.0.8 —
  delivery mechanism [OPEN] for 7B live verification); permissions/cancel/
  tools/shell/diffs largely mapped; gaps cluster at gates, history
  hydration/pagination, reconnect robustness; candidate ops routes
  (command/compact/diff/model/agent) need item-by-item approval.
* Frozen: Phase 5/6 untouched; Backend trait additive-only with mock
  parity; no new deps; Migration: NONE.
* Deliverables: `research/memory/phase7a-recon.md`,
  `research/memory/phase7a-decision-log.md` (7A-R1…R5). No `.rs` touched;
  `~/warp` untouched.
* Next: Phase 7B implementation — **start only on explicit user
  authorization** (recommended scope: gates + history/robustness).

### Phase 7 — Full Agent Interaction — ⏳ PLANNED

Complete whatever remains from the Phase 4 report: questions, permissions,
tools, shell, files, diffs, cancellation, errors, session history,
reconnect behavior.

### Phase 8 — Memory Intelligence — ⏳ PLANNED

Automatic extraction, consolidation, semantic/hybrid retrieval, entity
relationships, temporal reasoning, contradiction detection, confidence
scoring, decay, relevance scoring. Not prematurely.

### Phase 9 — TUI Memory UX — ⏳ PLANNED

Possible `/memory`, `/memory search`, `/memory show`, `/memory forget`;
visibility into memory used, source, scope, confidence, recency. UX not
finalized.

### Phase 10 — Production Hardening — ⏳ PLANNED

Performance, reconnects, persistence, crash recovery, migrations, OpenCode
version compatibility, security, memory limits, backup/export, testing.

## Important architectural decisions

* **TUI/backend separation**: the TUI must remain independent from
  OpenCode-specific types.
* **MockBackend**: stays available for deterministic testing and development.
* **OpenCode adapter**: all OpenCode communication/translation lives behind
  the adapter boundary.
* **Original Warp repository**: `~/warp` remains untouched.
* **Rendering**: Warp/warpui components used where permitted, with
  license/provenance preserved (see `NOTICE.md`, `research/licensing.md`).
* **Memory**: designed around OpenCode compatibility, not as a generic
  disconnected service.
* **Database**: do not replace or modify OpenCode's SQLite schema; memory
  storage follows from research.

## 🚫 DO NOT START YET

Until the Memory Research Phase is complete: do not implement the memory
database, embeddings, vector DB, automatic extraction, memory prompts; do
not alter OpenCode's SQLite schema; do not fork OpenCode; do not redesign
the TUI; do not replace the Backend architecture. Research first.

## Phase completion rule

Every phase entry contains objective, status, major work, files/research,
tests/verification, known limitations, architectural decisions, next phase.
A phase is complete only when its acceptance criteria are met — never merely
because code compiles.

## Phase History

```
Phase 1 — COMPLETE
Phase 2 — COMPLETE
Phase 3 — COMPLETE
Phase 4 — COMPLETE
Memory Research Phase 1 — COMPLETE
Memory Research Phase 2A — COMPLETE
Memory Research Phase 2B — COMPLETE
Phase 5 — COMPLETE
```

## Maintenance Rules

1. Update this file whenever a phase starts or finishes.
2. Never mark work complete without verification.
3. Record important architectural decisions.
4. Record known limitations instead of hiding them.
5. Do not silently change the roadmap.
6. If the architecture changes, document why.
7. Keep the roadmap synchronized with `research/`.
8. Keep phase boundaries explicit.
9. Do not start a future phase accidentally.
10. Preserve the distinction between research, implementation, testing, and polish.
