PHASE 7C IMPLEMENTATION REPORT

Implemented (all seven O-C operations + cross-cutting support):

- Compact: `POST /api/session/{id}/compact {}` admit → busy + Notice;
  lifecycle via `compaction.started/failed/ended` events + `compaction`
  history marker (failed → typed Error). No auto-compact; REST 200 is
  admission, not completion.
- Diff: `GET /api/session/{id}/diff` read-only; empty → "No differences."
  Notice; populated → summary Notice (file + status + counts) + `Edits`
  block with unified patch lines (headers skipped, 120 lines/file cap
  with `… (truncated)` marker). No session mutation, no fake events.
- Revert: stage → actual staged `files[]` shown as `Edits` (reverse
  patches) + Question confirm gate naming file restoration AND message
  deletion irreversibility; digit 1 → commit (deletes post-boundary
  messages, transcript re-hydrated from server truth); digit 2 →
  `DELETE /revert` abandon (messages kept). Never auto-stage/commit.
  Bad id → typed 404 Error; commit/abandon with no boundary → Error.
- Fork: `POST …/fork` ({} full / {before} scoped); bad `before` → 404
  Error; child registered (summary + lazy hydrate), parent + focus
  preserved, parent Notice names child; busy-gated (conservative).
- Model switching: `GET /api/model` discovery → client-side validation
  (unusable entries filtered); bogus ids rejected locally with available
  list, never POSTed; valid → 204 → Notice + info refresh.
- Agent switching: same shape vs `GET /api/agent`.
- Slash commands: `GET /api/command` discovery shown in existing menu;
  unknown name → Error, never POSTed; `init`-class writers (confirm-list:
  `init`; read-only: `review`; unknown → confirm) gate via Question
  digits; cancel → no POST; accept → exactly one POST → User echo + busy,
  existing turn pump handles execution.

Cross-cutting:
- Mapper: `compaction`/`agent-switched`/`model-switched` history types
  render (Error/Notice); `compaction.started/failed/ended`,
  `revert.committed` event arms; `idle` intentionally unmapped (no content).
- Event handling: existing 7B `connection.lost/restored`, busy, gates,
  cancel-clears-busy untouched; compact steers (no busy guard by design);
  fork/revert/exec busy-guarded; no mutation replay after reconnect.
- Validation: pure `validate_model`/`validate_agent` against cached
  discovery (fetch-once, failure caches nothing).
- Confirmations: revert + writer-commands reuse Question-gate digits
  (options-matched, so live agent questions are never eaten); zero TUI
  changes — all ops reachable by typing `/diff /compact /fork /revert
  /model /agent` or a discovered `/command`.
- MockBackend: all 9 trait methods + confirm-aware `answer_question` +
  7C slash text routing; deterministic catalogs (mock-sonnet;
  build/general; init gates, review runs).
- Reconnect: 7B system reused unchanged; commit re-hydrates from server.
- Error handling: every failure in-band; sessions never die.

Fork memory decision:
- 7C-R4 option (a): fork child = new session for memory injection.
  Children inherit history/agent/model but not entries (verified live:
  child entries-GET → 404, tolerated via `try_get_instruction_entry`);
  the child receives a fresh snapshot through the frozen Phase 6
  `inject_new_session` seam. No raw-entry copying, no architecture change.
  A tolerant entry check on fork warns if the server ever starts
  inheriting entries (assumption monitor, 7C-R2).

Revert safety model:
- Stage restores files immediately (server-side, verified); the gate
  shows the real staged `files[]` and states both destructive axes
  (files already restored + commit deletes messages irreversibly);
  commit only on digit 1; digit 2 abandons via DELETE; no silent path.

Command writer confirmation:
- Confirm-list `["init"]`, read-only `["review"]`, unknown defaults to
  confirm. Gate shows writability reason; accept POSTs exactly once;
  reject posts nothing.

Tests:
- cargo check --all-targets: clean.
- backend tests: all pass (client 14 stub tests incl. 9 new 7C wire-shape
  tests; mapper 18 incl. switch/compaction/revert fixtures; mod 19 pure +
  10 real-backend-vs-stub tests incl. bogus-never-sent asserting zero
  POSTs; mock 15 incl. 7 parity + slash-routing tests).
- full tests: 195 passed / 0 failed / 1 ignored (live read-only test).
- fmt: clean. clippy --all-targets: zero warnings.
- PTY: 18/18 green (mock backend incl. new slash routes).
- live verification (OpenCode 2.0.8, throwaway sessions, zero model turns
  burned, all deleted, user sessions untouched): compact admit 200 +
  typed failed marker; diff []; stage 200 + files:[] / bad id 404 /
  commit 204 / delete 204; fork full (child + title + boundary) / bad
  before 404; child entries-GET 404 (R2 real); model + agent switch 204.

Known non-blocking unknowns (declared, handled conservatively):
- Populated diff rendering live (R5-verified shape + fixture tests; only
  empty diff exercised live this phase — no turns burned for edits).
- Revert with real files live (R5-verified; this phase exercised the
  files:[] boundary + full gate matrix in stubs/mock).
- `review` exec live (R5-verified same {name,text} shape; not re-burned).
- Binary/truncated diffs (client cap + marker), revert/fork during
  generation (busy-guarded), compact success rewrite (failure path live),
  switch-dedicated SSE (history markers authoritative).

Files changed:
- src/backend/mod.rs (9 additive trait methods)
- src/backend/mock.rs (parity impls + slash routing + catalog + tests)
- src/backend/opencode/client.rs (11 thin wrappers + tolerant 404 read + tests)
- src/backend/opencode/mapper.rs (3 history types + 4 event arms + tests)
- src/backend/opencode/mod.rs (State fields, pure helpers, 9 impls,
  submit routing, confirm gates, backend-vs-stub tests)
- (pre-existing uncommitted 7B/6B tree left intact and untouched)

Dependencies: none added (std + existing serde_json/ureq only).

~/warp: clean, untouched (`git status --porcelain=v1` empty).

Phase 5/6/7B semantic changes: none. Memory schema/API/ordering/budget/
injection frozen and untouched; 7B gates/pagination/reconnect/cancel
paths verified intact (full suite + PTY green); TUI untouched.

Production issues: none. One leftover session check: a `ses_f407…`
session in the list is the user's own ("Make Lathe show in app drawer") —
not a probe leftover; both 7C throwaways deleted (204, verified absent).
