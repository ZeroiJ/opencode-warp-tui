PHASE 7B COMPLETE. HARD STOP. WAITING FOR PHASE 7C AUTHORIZATION.

## Scope

Authorized 7B work only: **O-A** (agent question gates — answer delivery via
the verified form-reply route + permission polish) and **O-B** (session
history pagination + reconnect robustness). Everything else (O-C, memory UI,
Phases 8–9) untouched.

## Environment & Baseline

- Server under test: live OpenCode **2.0.8** managed service at
  `http://127.0.0.1:49374` (PID 4922), Basic auth from `service.json`
  (password never printed or committed).
- Baseline recorded before 7B code: fmt / check / clippy clean;
  `cargo test` **151 passed / 0 failed / 1 ignored**; PTY probe **18/18**;
  build clean; warp checkout clean. (`/tmp/opencode-warp-tui-baseline.log`)
- Baseline source of truth for wire shapes: `research/memory/phase7b-question-probe.md`
  (live probe; capture files `/tmp/owt-probe-events.jsonl`,
  `/tmp/owt-probe-lifecycle.jsonl`; probe modules `/tmp/probe_question.py`,
  `/tmp/probe_lifecycle.py`). Both throwaway probe sessions deleted.

## Implementation Summary

### Files Changed (5 existing)

- `src/backend/opencode/client.rs` — added `reply_form()` (POST
  `/api/session/{sid}/form/{formID}/reply`, dynamic field key via
  `serde_json::Map`, sends the option **value**); added
  `list_messages_paged()` (cursor-based `limit`/`order=asc`/`cursor` query,
  no `offset` — live 2.0.8 pagination); removed the now-unused
  `list_messages`. Stub-server test module: 4 new tests (reply form body
  + path, 400 rejection surfaces as error, cursor query build, asc default).
- `src/backend/opencode/events.rs` — `SseWorker` now emits synthetic
  `connection.lost` (stream EOF/error) and `connection.restored`
  (successful reconnect) `RawEvent`s, one per drop cycle; existing
  backoff/reconnect preserved; `send_connection_event` helper; worker
  test still green.
- `src/backend/opencode/mapper.rs` — `map_form_created()`: `form.created`
  envelopes with `metadata.kind == "question"` render one
  `Block::Question` per question field (prompt from field title/description,
  labels from `options[].label`); non-question forms map to nothing;
  wired into `map_event`. 2 new tests.
- `src/backend/opencode/mod.rs` — promoted `_pending_questions` →
  `pending_questions: HashMap<String, Vec<PendingQuestion>>` +
  `connection_gap: bool`; `PendingQuestion { form_id, field_key, option_values }`;
  ingest extracted into testable `apply_raw()` (busy tracking unchanged;
  `form.created`/`form.replied`/`form.rejected`/`question.v2.*`/
  `connection.lost`/`connection.restored` arms); `session_of()` reads
  nested `form.sessionID`; `pending_from_form()` helper; `hydrate_active()`
  rewritten to cursor-based `order=asc` pagination following `cursor.next`
  (`MAX_HISTORY_PAGES = 50`); `answer_question()` rewritten (real
  `reply_form` delivery, 204 → pop trailing gate + `Answered: {label}`
  Assistant block; gate without a live form → local resolve with visible
  "Chose … no live question form" notice; out-of-range option → Error,
  gate kept; wire failure → visible "Answer was NOT delivered" Error,
  gate kept). 5 new unit tests; 1 ignored live test unchanged.

### Files Added (1 new, uncommitted from previous phases)

- `src/backend/opencode/memory_inject.rs` (Phase 6B, unchanged by 7B)

## What Was Implemented

**O-A — question gates (verified delivery path):**

```
question tool called by the agent
        │
        ▼
SSE: form.created  (metadata.kind == "question", frm_ id, per-field key/options)
        │
        ├─► mapper: one Block::Question per field (digits already dispatch
        │     SessionAction::AnswerQuestion(0/1/2) — no TUI change needed)
        ├─► apply_raw: record PendingQuestion { form_id, field_key, option_values }
        │
        ▼
user presses 1/2/3  →  answer_question(option)
        │
        ├─► option in range?
        │     ├─ no  → in-band Error, gate kept (refuse politely)
        │     └─ yes → pending live form?
        │              ├─ no  → in-band "Chose … no live question form"
        │              │        (gate resolved locally, like permission fallback)
        │              └─ yes → POST /api/session/{sid}/form/{formID}/reply
        │                        body {"answer": {<field_key>: <option VALUE>}}
        │                        ├─ 204 → pop trailing gate + "Answered: {label}";
        │                        │         `form.replied` clears the form's pendings
        │                        └─ Err → in-band "Answer was NOT delivered" Error,
        │                                  gate kept for retry
        ▼
agent receives the choice (verified: tool state "completed",
      metadata.answers: [["value"]], then session.execution.succeeded)
```

- Single-question forms (the verified live case) answer the trailing gate;
  multi-field forms answer one gate at a time (trailing gate = last field,
  `form.replied`/`form.rejected` clears remaining entries).
- History rendering: completed `question` tool calls hydrate as done tools
  (never re-gates); only a **running** question on the live feed surfaces a
  gate — answered gates never re-appear.

**O-B — session history & reconnect robustness:**

- `hydrate_active` uses `?limit=N&order=asc` + follows `cursor.next`
  (live 2.0.8 is cursor-based, default wire order is desc/newest-first —
  the old hydrate rendered the transcript inverted). History is re-based
  from REST after any reconnect (live-only SSE, no replay).
- `connection.lost`/`connection.restored` synthetic worker signals:
  dropped/failed stream clears stale `busy` flags (statusline never spins
  forever) and surfaces the gap in-band (Notice on the active transcript),
  one notice per drop cycle (`connection_gap` latch).
- `cancel()` clears `busy` even when the interrupt request fails (logs the
  failure) — the UI never wedges on a rejected interrupt.

## Verification Results

| Check | Result |
|---|---|
| `cargo fmt --check` | ✅ clean |
| `cargo check --all-targets` | ✅ zero errors |
| `cargo clippy --all-targets` | ✅ zero warnings |
| `cargo test` | ✅ **162 passed / 0 failed / 1 ignored** (baseline 151 → +11 new) |
| `scripts/pty_probe.py ./target/debug/owt` | ✅ 18/18 checks green |
| `git -C /home/zeroij/warp status --porcelain=v1` | ✅ clean (untouched) |

New tests (11): mapper 2 (`question_v2_gate_and_form_created_share_router`,
`non_question_forms_map_to_nothing`); client 4 (reply_form path+body,
reply_form 400 → error, list_messages_paged cursor query, asc default);
mod.rs 5 (`session_of_reads_form_nested_id`,
`pending_from_form_parses_question_fields`,
`apply_raw_records_and_clears_question_gates`,
`apply_raw_reconciles_connection_gap_busy`,
`apply_raw_question_v2_clears_pending_on_reply`).

## Diff Hygiene

| Check | Result |
|---|---|
| TUI changes | ✅ none (digit dispatch `AnswerQuestion(0/1/2)` pre-existed, session.rs:599-605) |
| Backend trait changes | ✅ none (`answer_question(usize)` existed) — additive-only, MockBackend parity kept |
| Warp (`~/warp`) modifications | ✅ none |
| New Cargo dependencies | ✅ none (std + existing `serde_json`, `ureq` only) |
| Memory frozen (7A-R3) | ✅ memory schema/store/API, ordering, budget, injection format, `owt.memory`, snapshot/resume, probe behavior — none touched |
| New global manager | ✅ none (`pending_questions` lives on the existing `State`) |
| Duplicate protection / event DB | ✅ none added (existing session-id based dedup only) |
| New user ops (compact/diff/fork/revert/model/agent switch/slash) | ✅ none |
| `permission` "always" surfacing | ✅ absent (keep once / reject only, as authorized) |
| New memory architecture / SQLite / embeddings / FTS5 / vector DB | ✅ absent |
| Provider-specific paths | ✅ absent |

## Probe Documentation

- `research/memory/phase7b-question-probe.md` — authoritative live probe
  write-up (resolves 7A-R2): `form.created` envelope shape, verified
  `POST …/form/{formID}/reply` delivery, `metadata.answers` proof,
  lifecycle ordering, throwaway-session cleanup.
- `research/memory/phase7a-decision-log.md` — appended **7A-R2 RESOLVED**
  note citing the probe and the chosen mechanism, with impact statement.

## Remaining Issues

None within Phase 7B scope. OpenCode integration remains isolated from the
TUI layer per AGENTS.md; memory injection is untouched; no O-C / session-ops /
memory-UI / Phase 8–9 work was performed.

The final line of this report must be:

```
PHASE 7B COMPLETE. HARD STOP. WAITING FOR PHASE 7C AUTHORIZATION.
```