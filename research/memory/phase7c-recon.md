# Phase 7C — Advanced Agent Operations Research

> Status: 🔬 RESEARCH ONLY. **No production code modified, no behavior
> changed, no dependencies added, `~/warp` untouched.** Evidence from
> read-only source inspection, a read-only live OpenAPI fetch, and
> controlled live probes against the local OpenCode 2.0.8 server using
> throwaway sessions (4 created, 4 deleted, all 204; zero leftovers).
> One model turn was burned (PONG, no tools); all other probes were
> REST-only.
>
> Labels: **[VERIFIED]** probed live this phase · **[SOURCE]** OWT/OpenCode
> source or live spec read · **[INFERRED]** reasoned conclusion ·
> **[DOCUMENTED]** project/official docs · **[UNVERIFIED]** not established.
>
> Companion: `phase7c-decision-log.md` (7C-R1…R8). Baselines: Phase 7A
> recon, Phase 7B commit `a32354d` + uncommitted 7B tree (read, not
> touched), Phase 5/6 frozen.

---

## 1. Executive summary

All seven O-C operations **exist** on OpenCode 2.0.8 with documented
REST contracts, and six of seven had their success paths verified live.
Three findings matter most for the implementation phase:

1. **The server validates almost nothing**: model/agent switches accept
   bogus values (204) and persist them verbatim; the failure surfaces
   later as a dead turn (`outcome: failed`, zero tokens, no messages).
   OWT must validate against discovery lists client-side (7C-R3).
2. **History is append-only everywhere observed**: switches materialize
   as `agent-switched`/`model-switched` messages; failed compactions
   append a `compaction` message with typed status; revert ops never
   delete messages (one `idle` marker appended). But OWT's mapper
   currently **drops all of these types on hydrate** — rendering them is a
   7C-implementation requirement (7C-R6).
3. **Fork does not inherit everything**: children get history + agent +
   model, but **not** instruction entries and **not** the staged revert
   boundary. The entries-GET route also 404s (vs 200 `[]`) on a fork
   child with zero entries — the tolerant reader must treat 404 as empty
   (7C-R2).

Readiness: compact / fork / model-switch / agent-switch → READY FOR
IMPLEMENTATION. Diff (populated shape), revert (with real file changes),
slash-command execution → NEEDS MORE RESEARCH (each needs exactly one
targeted probe increment). Nothing → DEFER outright, but scope decisions
remain with the authorizer.

## 2. Environment/version

| Item | Value |
|---|---|
| OpenCode server | **2.0.8** (`GET /api/info`, pid 33013; restarted since 6A) **[VERIFIED]** |
| OWT | `a32354d` + uncommitted 7B tree (implementation read, untouched) |
| Rust | 1.92.0 / cargo 1.92.0 |
| Provider/model | `opencode`/Zen; `mimo-v2.5-free` (one PONG turn, no tools) |
| Agents discovered | 15 (`build` default + `general`, `explore`, `compaction`, `title`, `summary`, `plan`, `librarian`, `metis`, `momus`, `oracle`, `atlas`, `hephaestus`, `prometheus`, `multimodal-looker`) **[VERIFIED]** |
| Models discovered | multiple free (`mimo-v2.5-free`, `muse-spark-1.3…`, `grok-…`, …) via `GET /api/model` **[VERIFIED]** |
| Server commands | `init` (guided AGENTS.md setup — **writes files**), `review` (reviews changes) via `GET /api/command` **[VERIFIED]** |
| Auth | `Basic opencode:<password>` (service registration) |
| Spec | OpenAPI 3.1.0, 113 paths, fetched live to `/tmp` (outside repo) **[SOURCE]** |

## 3. Existing OWT architecture relevant to O-C

- `Client::request()` (blocking ureq, 15 s, tolerant `Value`, 300-char
  error bodies) already speaks every verb needed; 7B added
  `list_messages_paged`, `reply_form`, `reply_permission`,
  instruction-entry wrappers **[SOURCE]**.
- `Backend` trait + MockBackend parity pattern; `State` holds sessions,
  busy, pending gates; 7B added cursor pagination, `connection.lost` /
  `restored` synthetics, busy reconciliation, cancel-clears-busy
  **[SOURCE]**.
- SSE worker: live-only, no replay, backoff reconnect; state re-based
  from REST after reconnect **[SOURCE]** `events.rs`.
- Mapper gaps found this phase (**new behavior discovered**): `map_message`
  drops `compaction`, `agent-switched`, `model-switched`, `idle`
  (`_ => vec![]`); `map_event` ignores compaction/revert/synthetic/switch
  events (comment at `mapper.rs:~421`) **[SOURCE]**.

## 4. Compact research

- **Route**: `POST /api/session/{id}/compact`, body `{}` (`{id: msg_?,
  delivery?}` optional; default steers at next step boundary)
  **[SOURCE + VERIFIED]**.
- **Sync/async**: async. REST 200 immediately admits a `type:
  "compaction"` message (`delivery: "steer"`); execution runs through the
  inbox. **[VERIFIED]**
- **Events** (S1-scoped, time-ordered from live SSE capture)
  **[VERIFIED]**:
  `session.inbox.enqueued → session.execution.started →
  session.inbox.delivered → session.compaction.started (reason:"manual") →
  session.compaction.failed (reason:"manual",
  error:{"type":"compaction.unavailable","message":"Nothing to compact
  yet"}) → session.execution.succeeded`.
- **History**: failed compaction appends the `compaction` message with
  `status:"failed"`; user/assistant messages untouched
  **[VERIFIED]**. Populated-history compaction success path
  **[UNVERIFIED]** (history too small in every probe).
- **Cross-op**: compact **consumed the staged revert boundary** —
  `session.revert.committed` emitted during the compact window and the
  session `revert` field went staged → `None` **[VERIFIED]**.
- **Gates/reconnect/cancel interplay**: not probed live (no gate active;
  compaction steers behind queued prompts per docs). 7B's generic
  busy-reconciliation covers the busy side **[INFERRED]**.
- **Failure shape**: typed `error:{type, message}` on the message +
  terminal `compaction.failed` event **[VERIFIED]**.

## 5. Diff research

- **Route**: `GET /api/session/{id}/diff?from=&to=&context=`; turn-scoped
  file snapshots (first recorded snapshot → last; running step compares
  working copy) **[SOURCE]**.
- **Probed**: empty session → 200 `{"data":[]}`; post-turn with no file
  changes → 200 `{"data":[]}` **[VERIFIED]**.
- **Populated diff shape UNVERIFIED** — no probe made file changes, so
  per-file add/modify/delete representation, binary handling, truncation,
  and pagination were not observed **[UNVERIFIED]**.
- Read-only: no events, no history mutation, no session impact
  **[INFERRED from GET semantics + zero observed side effects]**.

## 6. Revert research

- **Model**: message-anchored boundary + optional file restoration — NOT a
  pure filesystem undo and NOT history deletion. Documented distinction:
  `revert/stage` records `{messageID, files[]}` server-side (visible in
  `GET session` → `revert` field); messages are never deleted in any
  observed op **[VERIFIED]**.
- **Stage**: `POST …/revert/stage {"messageID": msg_}` (required) → 200
  `{data:{messageID, files:[]}}`; no message trace appended; bad id →
  404 `MessageNotFoundError` (typed) **[VERIFIED]**.
- **Commit**: `POST …/revert/commit` (no body) → 204; with no staged
  boundary, silent no-op **[VERIFIED]**.
- **Delete**: `DELETE …/revert` (no body) → 204; after a commit+delete
  sequence one new `idle` marker appeared, zero messages removed
  **[VERIFIED]**.
- **Filesystem restore UNVERIFIED**: `files:[]` in every probe (PONG turn
  touched nothing); non-empty restore behavior, reversibility of applied
  restores, and permission interplay were not observed **[UNVERIFIED]**.
- Active-generation / gate-active interplay **[UNVERIFIED]**.

## 7. Fork research

- **Route**: `POST …/fork {before?: msg_}` → 200 new session
  **[VERIFIED]**. Omit `before` → full-history copy (`boundary.type:
  "through"`); with `before` → `boundary.type: "before"`;
  `fork:{sessionID: parent, boundary}` on the child; title `"<parent>
  (fork #1)"` (counter observed static across two forks — server quirk)
  **[VERIFIED]**. Bad `before` → 404 `MessageNotFoundError` **[VERIFIED]**.
- **Copied**: projected history (incl. `agent-switched`/`model-switched`
  markers), agent, model (even bogus values — verbatim)
  **[VERIFIED]**. Cost/tokens reset to 0; new `created` time
  **[VERIFIED]**.
- **NOT copied**: staged revert boundary (`None` on child), instruction
  entries (child has none) **[VERIFIED]**.
- **Memory interaction** (frozen rules preserved): since entries don't
  transfer, a forked session has no `owt.memory` until something writes
  it. Whether a future fork implementation should inject the current
  corpus into the child is an implementation-scope decision (options in
  decision log; default recommendation: treat child like a new session)
  — no architecture change either way (7C-R4).
- Fork-during-generation **[UNVERIFIED]** (no active turn available
  without burning a tool-capable turn).

## 8. Model switching research

- **Route**: `POST …/model {"model":{"id","providerID"}}` (both required;
  `variant?`) → 204, no body **[SOURCE + VERIFIED]**.
- **Validation gap (key finding)**: bogus `{id,providerID}` → **204**,
  persisted verbatim on `session.model`; a subsequent turn died
  (`outcome:"failed"`, zero tokens, **no messages at all** — not even the
  user message or an error) **[VERIFIED]**. OWT must validate against
  `GET /api/model` client-side (7C-R3).
- **History**: switches materialize as `model-switched` messages, copied
  by fork **[VERIFIED]**.
- **Scope**: "subsequent provider turns" per docs; current-turn switch
  **[UNVERIFIED]**.
- Dedicated switch SSE event **[UNVERIFIED]** (markers seen only via
  message polling; no switch-specific event isolated in capture).
- Persistence across reload: switch persisted on session object
  (survives re-GET; restart-reload **[UNVERIFIED]** but server-side state
  implies yes **[INFERRED]**).

## 9. Agent switching research

- **Route**: `POST …/agent {"agent": "<id>"}` → 204, no body; same
  zero-validation behavior as model (bogus agent persisted verbatim)
  **[VERIFIED]**.
- "Agent" = named preset (id, model binding, permissions, instructions);
  15 discovered; `build` is default primary **[VERIFIED]**.
- History markers `agent-switched`; fork-inherited **[VERIFIED]**.
- Instruction/tool/system-behavior deltas per agent **[UNVERIFIED]**
  (would need per-agent turns).
- Same validation requirement as model (validate vs `GET /api/agent`)
  (7C-R3).

## 10. Slash-command research (caution exercised)

- **Separation**: OWT-local `/memory` (adapter-intercepted, no network)
  vs server commands (`init`, `review` via `GET /api/command`) vs
  execution route (`POST …/command {name, text, files?, agents?,
  skills?, delivery?}`, both required) vs prompt text resembling a
  command — four distinct things, not interchangeable **[SOURCE]**.
- **Discovery VERIFIED**; **failure VERIFIED**: bogus name → 404
  `{"_tag":"CommandNotFoundError","command":…}` **[VERIFIED]**.
- **Success execution UNVERIFIED-live, deliberately**: `init` writes
  workspace files (destructive-adjacent), `review` burns an agent turn;
  neither was executed against this repo state. Recommended next probe:
  controlled `review` on a scratch repo (7C-R5).
- Permission/history/event behavior of execution **[UNVERIFIED]**.
- If no success path is ever verified: there IS a verified API shape, so
  the honest label is NEEDS MORE RESEARCH, not NO-API.

## 11. Event matrix

| Operation | Event(s) observed | Ordering | Terminal | Error event |
|---|---|---|---|---|
| Compact | inbox.enqueued, execution.started, inbox.delivered, compaction.started, compaction.failed, execution.succeeded | REST 200 first, then SSE sequence above | compaction.failed (here) / `.ended` per mapper vocab | compaction.failed + typed message error |
| Diff | — (read-only GET) | N/A | N/A | N/A |
| Revert stage | none isolated | N/A | N/A | 404 typed (REST) |
| Revert commit | `session.revert.committed` (observed in compact window; stage/commit not individually isolated) | UNVERIFIED exact placement | UNVERIFIED | REST 4xx |
| Revert delete | none isolated | UNVERIFIED | UNVERIFIED | REST 4xx |
| Fork | none captured (forks predated SSE capture) | UNVERIFIED | N/A (immediate 200+child) | 404 typed (REST) |
| Model switch | history marker only (no dedicated event isolated) | UNVERIFIED | N/A (immediate 204) | none (accepts blind) |
| Agent switch | same as model | UNVERIFIED | N/A | none (accepts blind) |
| Slash exec | UNVERIFIED | UNVERIFIED | UNVERIFIED | 404 typed (REST) |

REST always precedes SSE; nothing observed suggests SSE-only inference is
possible for these ops — OWT should drive them from REST responses and
treat events as confirmation **[INFERRED]**.

## 12. History impact (per-operation lifecycle)

| Operation | History change observed |
|---|---|
| Compact (failed) | appends `compaction` msg w/ `status:"failed"`; rest intact |
| Compact (success) | UNVERIFIED (expect summary replacement per docs — do not assume) |
| Diff | none (read-only) |
| Revert stage/commit/delete | none deleted; +1 `idle` marker across commit+delete sequence; boundary in `session.revert`, not messages |
| Fork | parent untouched; child starts with copied projected history |
| Model/agent switch | appends `model-switched` / `agent-switched` marker |
| Slash exec | UNVERIFIED |

`GET …/message` (cursor-paged, 7B) reflects all of the above; reconnect
reproduces the same state via re-hydrate **[INFERRED from 7B infra]**.

## 13. Reconnect impact

- No new reconnect machinery needed: all O-C state (switches, revert
  boundary, fork children as sessions, compaction markers) is readable
  via `GET session` + paged messages — the 7B rebase path covers them
  **[INFERRED]**. 
- Missed terminal events (e.g. `compaction.failed` during a gap):
  reconciled by re-reading messages + session object on `restored`
  (markers + `outcome` + `revert` fields carry the truth)
  **[INFERRED]**.
- Duplicate protection: ops are idempotent-by-construction except fork
  (creates a child per call — Fork requires explicit user gesture + the
  returned child id recorded before any retry) and command-exec
  (UNVERIFIED idempotency — must not auto-retry) **[INFERRED]**.
- Busy: compact runs server-side execution (busy window observed);
  7B busy-reconciliation applies unchanged **[INFERRED]**.

## 14. Cancellation impact

- Compact steers at the next step boundary (does not preempt a running
  step) — cancel-during-compact **[UNVERIFIED]** live; `interrupt` remains
  the generation-stop primitive **[SOURCE + INFERRED]**.
- Diff/fork/switches are immediate (no cancellable window)
  **[VERIFIED]**.
- Revert ops immediate in probes; revert-during-generation
  **[UNVERIFIED]**.
- No stale-busy evidence in any probe (sessions returned to idle)
  **[VERIFIED]**.

## 15. Security/safety considerations

| Operation | Risk | Requirement for implementation |
|---|---|---|
| Compact | summarization discards detail (server-side, standard) | confirm: explicit user action; show status |
| Diff | none (read-only) | none |
| Revert **with files** | filesystem restoration = destructive-adjacent | **explicit confirmation** + show affected files first (from staged `files[]`); never auto-revert (7C-R7) |
| Fork | negligible (new session) | none beyond explicit gesture |
| Model/agent | dead-turn footgun via invalid values | client-side validation vs discovery (7C-R3); invalid selection not sendable |
| Slash exec | arbitrary agent turn; `init` writes files | confirmation, at least for file-writing commands; surface command description from discovery |

Secret handling unchanged (routes carry ids/options; memory content never
involved). No new auth surface (existing client auth reused).

## 16. Failure matrix

| Operation | Success | Failure | Events | History | Reconnect | Safe to implement |
|---|---|---|---|---|---|---|
| Compact | YES (admitted; fail-path verified) | YES (typed) | YES | YES (fail-path) | UNVERIFIED (covered by 7B infra — INFERRED) | YES |
| Diff | YES (empty) | YES (404) | N/A | N/A | N/A | note §23 (populated shape UNVERIFIED) |
| Revert | YES (no-op semantics) | YES (404s) | PARTIAL | YES (append-only) | INFERRED | note §23 (files path UNVERIFIED) |
| Fork | YES | YES (404) | UNVERIFIED | YES | INFERRED | YES |
| Model switch | YES (204+persist+marker) | YES (blind-accept gap) | PARTIAL | YES | INFERRED | YES (with validation) |
| Agent switch | YES (204+persist+marker) | YES (blind-accept gap) | PARTIAL | YES | INFERRED | YES (with validation) |
| Slash commands | UNVERIFIED-live | YES (404 typed) | UNVERIFIED | UNVERIFIED | UNVERIFIED | note §23 |

## 17. Implementation readiness

**Compact — READY FOR IMPLEMENTATION.**
`POST …/compact {}`; admit-message → poll markers/events → terminal
`compaction.failed/.ended`; render status; failures typed. Needs: client
wrapper, mapper additions (compaction message + events), busy cover,
confirm-before-compact (destructive-adjacent perception). Risks: success-
path history rewrite unobserved (small-history servers fail fast; a real
compaction needs a long session — state this in scope). Unknowns: gate-
active + cancel-during-compact interplay.

**Diff — NEEDS MORE RESEARCH.**
One probe increment: make a real file change in a throwaway worktree
session, then GET diff and record the populated shape (per-file
representation, binary/truncation/pagination). Fetch plumbing + empty
state are ready; rich rendering is not.

**Revert — NEEDS MORE RESEARCH.**
One probe increment: stage/commit with non-empty `files[]` in a
throwaway worktree; observe filesystem outcome, reversibility, events.
Contract must include confirmation (7C-R7). Current no-op semantics are
fully verified and safe to ship as boundary-setting alone — authorizer
call.

**Fork — READY FOR IMPLEMENTATION.**
`POST …/fork {before?}`; register child session (title, select? —
authorizer call); no memory/revert inheritance (decide injection per
7C-R4); fork events unobserved but non-blocking (REST carries the child).
Risks: fork-during-generation unverified → disableながらbusy (recommended).
Unknowns: entry-GET 404 quirk (handled by tolerant read, 7C-R2).

**Model switching — READY FOR IMPLEMENTATION (with validation).**
Client MUST validate `{id, providerID}` against `GET /api/model`
(disabled/excluded entries respected); switch → 204 → marker confirms.
Risks: blind-accept footgun (eliminated by validation); current-turn
scope unverified. Unknowns: restart persistence (expected yes).

**Agent switching — READY FOR IMPLEMENTATION (with validation).**
Same shape vs `GET /api/agent`. Unknowns: per-agent behavior deltas
(describe from discovery metadata only, don't claim more).

**Slash commands — NEEDS MORE RESEARCH.**
One probe increment: controlled `review` execution on a scratch repo
(events, history, permissions, failure). Discovery + failure shape done.
`init` must never be executed implicitly (writes files).

## 18. Proposed Phase 7C implementation boundary

In: client wrappers (compact/diff/fork/model/agent + revert trio),
mapper additions for `compaction`/`agent-switched`/`model-switched`
history types + compact/revert events, additive Backend trait methods
with MockBackend parity, TUI affordances reusing gate/menu/key seams,
client-side model/agent validation, fork child registration, entries-GET
tolerant 404, confirmation for revert-with-files + file-writing commands.
Out: populated-diff rendering (pending probe), revert-restore execution
(pending probe), command execution (pending probe), auto-compact policy,
any Phase 5/6/7B semantic change, new deps, persistence/migrations.

## 19. Explicit non-goals

Automatic memory, memory UI (Phase 9), intelligence (Phase 8),
TUI redesign, new persistence, OpenCode-side changes, Warp changes,
populated-diff UI, revert-restore, slash-exec — until their probes land.

## 20. Open questions

1. Populated diff shape (one worktree probe).
2. Revert restore semantics with real files (one worktree probe).
3. Slash-exec event/history shape (one scratch-repo `review` probe).
4. Successful-compaction history rewrite (needs a long session; may stay
   UNVERIFIED with documented assumption).
5. Fork-during-generation behavior (advise disable-while-busy regardless).
6. Entries-GET 404 exact trigger (quirk documented; tolerant read covers
   it — no further probe required).
