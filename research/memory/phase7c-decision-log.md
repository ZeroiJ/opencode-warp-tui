# Phase 7C — Decision Log

> Research only. No production change. Nothing here revises Phase 2B
> (D1–D25), Phase 5/6, or Phase 7B. 7C-R2 refines the 6A-R9 watch-item
> with new evidence (refinement, not a revision).

## 7C-R1 — No foundation changes required

- **Decision:** O-C implementation needs no Phase 5/6/7B semantic change.
- **Evidence:** All seven ops attach via new client wrappers + additive
  trait methods + mapper additions; probes never required touching
  memory, injection, gates, or reconnect paths.
- **Rationale:** The Backend-trait/MockBackend/client/mapper seams hold.
- **Alternatives:** None needed. **Consequence:** 7C implementation is
  purely additive (plus mapper rendering of new history types).

## 7C-R2 — Entries-GET 404 means empty (tolerant read)

- **Decision:** Treat instruction-entries GET 404 as "no entries".
- **Evidence:** Fork child with zero entries → GET 404 (×2); after PUT →
  200 with data; non-fork fresh session → 200 `[]`; S1 post-turn → 200
  with data. Plus the single 6A post-turn empty-body read.
- **Rationale:** Phase 4 tolerant-reader discipline; 404-vs-`[]` is a
  server quirk, not a signal. Refines (not revises) 6A-R9.
- **Alternatives:** Strict 404-as-error (would falsely disable memory on
  fork children). **Consequence:** One-line tolerance in future
  entries-reading code; Phase 6 paths (PUT/DELETE) unaffected.

## 7C-R3 — Client-side validation for model/agent switches (mandatory)

- **Decision:** OWT must validate selections against `GET /api/model`
  and `GET /api/agent` before switching; invalid values are never sent.
- **Evidence:** Bogus model/agent → 204, persisted verbatim; subsequent
  turn died silently (`outcome: failed`, zero tokens, no messages).
- **Rationale:** The server's blind accept turns a typo into a dead
  session with no user-visible error. Validation is the only guardrail.
- **Alternatives:** Send-and-observe-outcome (leaves dead turns;
  rejected). **Consequence:** Discovery caching + disabled-entry
  handling become implementation requirements.

## 7C-R4 — Fork-child memory/revert: decide at implementation scope

- **Decision:** No architecture change; the implementation authorizer
  picks: (a) inject current corpus into fork children (child treated as
  a new session), or (b) leave children uninjected (strict frozen-lineage
  reading).
- **Evidence:** Children inherit history/agent/model but not entries or
  staged revert boundaries (verified).
- **Rationale:** Both readings are defensible; the frozen rule governs
  modification of existing sessions, and a fork child is arguably new.
- **Alternatives:** (a)/(b) above. **Consequence:** Recommendation is
  (a) with a one-line rationale recorded at implementation time.

## 7C-R5 — Three NEEDS-MORE-RESEARCH increments specified

- **Decision:** Diff-populated-shape, revert-with-files, and slash-exec
  each need exactly one targeted probe (worktree file change; worktree
  restore; scratch-repo `review`) before implementation. `init` is never
  executed implicitly.
- **Evidence:** All three verified at boundary (empty/no-op/failure)
  but not at payload (populated diff, real restore, real execution).
- **Rationale:** Rendering/executing unverified payloads risks wrong UI
  and destructive behavior.
- **Alternatives:** Implement on assumed shapes (rejected).
  **Consequence:** Implementation phase splits into ready-now
  (compact/fork/switches) vs probe-then-build (diff/revert/exec).

## 7C-R6 — Mapper must render new history types (additive)

- **Decision:** Future implementation adds `compaction`,
  `agent-switched`, `model-switched` (and `idle` if desired) to
  `map_message`, plus compact/revert event arms in `map_event`.
- **Evidence:** Switches/compactions materialize as history messages
  that hydrate renders as nothing today (`_ => vec![]`).
- **Rationale:** Otherwise fork/switch/compact users see transcript
  holes on every session switch.
- **Alternatives:** None (pure gap). **Consequence:** Small additive
  mapper change with fixture tests; no event-schema redesign.

## 7C-R7 — Revert-with-files requires explicit confirmation

- **Decision:** Any future revert-restore execution must confirm with
  the affected file list shown (from staged `files[]`) and never
  auto-run.
- **Evidence:** Staged boundary exposes `files[]`; restore semantics
  with real files are unverified, hence destructive-adjacent.
- **Rationale:** Filesystem restoration is the only O-C op that can
  destroy user work; confirmation is the proportional guardrail.
- **Alternatives:** Auto-restore (rejected). **Consequence:** Contract
  item for the implementation phase, not this one.

## 7C-R8 — Compact/fork/switches are READY; scope stays with authorizer
- **Decision:** Rate compact/fork/model/agent READY FOR IMPLEMENTATION
  (with R3 validation + R6 rendering + stated unknowns); diff/revert/
  exec NEEDS MORE RESEARCH. No ranking, no scores, no DEFER labels —
  deferral is the authorizer's call.
- **Evidence:** Failure matrix §16 (only neutral YES/NO/UNVERIFIED/N/A).
- **Rationale:** The brief forbids ranking; readiness categories are
  mechanical readings of the matrix.
- **Consequence:** Implementation can start on the ready set while
  probes proceed in parallel.

## 7C-R9 — Populated diff shape verified → diff is READY

- **Decision:** Diff moves NEEDS MORE RESEARCH → READY FOR
  IMPLEMENTATION with the R5-1 contract.
- **Evidence:** `phase7c-r5-probes.md` R5-1: per-file `{file, patch
  (unified), additions, deletions, status}`; empty → `{"data":[]}`.
- **Rationale:** Shape is fully renderable; only binary/truncation edge
  behavior is unobserved (covered by a client-side patch cap).
- **Alternatives:** None. **Consequence:** Diff UI work unblocked.

## 7C-R10 — Revert stage/commit/delete semantics verified → revert is READY with confirmation

- **Decision:** Revert moves NEEDS MORE RESEARCH → READY FOR
  IMPLEMENTATION under the R5-2 contract; 7C-R7 stands strengthened.
- **Evidence:** `phase7c-r5-probes.md` R5-2: stage restores files
  immediately + records boundary; commit deletes post-boundary messages
  (404-verified) irreversibly; DELETE clears + appends idle, deletes
  nothing.
- **Rationale:** Both destructive axes are now characterized; the
  confirmation must name file restoration AND message deletion.
- **Alternatives:** Ship boundary-setting only (still permitted as a
  reduced scope). **Consequence:** Full revert flow unblocked behind
  explicit confirmation.

## 7C-R11 — Slash-exec success verified → commands are READY with writer confirm-list

- **Decision:** Slash execution moves NEEDS MORE RESEARCH → READY FOR
  IMPLEMENTATION under the R5-3 contract.
- **Evidence:** `phase7c-r5-probes.md` R5-3: 204 → synthetic user message
  + normal tool turn; `review` read-only confirmed; no uninvited writes.
- **Rationale:** Execution is turn-equivalent (existing pump covers it);
  the only new risk is file-writing commands, handled by a confirm-list
  starting with `init`.
- **Alternatives:** None. **Consequence:** Command execution unblocked;
  `init`-class commands always confirm.
