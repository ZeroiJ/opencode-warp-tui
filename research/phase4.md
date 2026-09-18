# Phase 4 — OpenCode adapter foundation

## OpenCode version/commit investigated

* Binary `opencode` 1.18.31 (mise, anomalyco lineage, single Bun binary).
* **Two API generations in play**: the managed background service in this
  environment runs **2.0.1** (V2 line); fresh `serve` from the 1.18.31 binary
  speaks an older dialect. Verified live per call — see
  `research/opencode-architecture.md`.
* Source (read-only): `sst/opencode` `packages/schema/src/*`,
  `packages/opencode/src/{bus/global.ts,session/}`.
* Contract: `/v2/openapi.json` (OpenAPI 3.1.0, 113 paths, 242 schemas).

## Architecture discovered

Client-server HTTP + SSE. Sessions (`ses_*`, parent/child via `parentID`,
token/cost accounting), message models (V2 `content[]` REST + V1 `parts[]`
export), full event vocabulary (`session.*` live on 2.0.1, `session.next.*`
in source; permission/question v2; `session.idle/status`), permission model
(tool-keyed, once/always/reject), question model (multi-question, label
options, answers), tool lifecycle (called/input-stream/success/failed +
content), shell events + `/api/shell*`, interrupt-per-session, Basic
`opencode:<password>` auth, `service.json` discovery, per-location scoping.

## Files created/modified

* `src/backend/opencode/{mod,config,client,events,mapper}.rs` (new adapter).
* `src/backend/{mod.rs (trait),stream.rs (new shared applier),mock.rs}`.
* `src/main.rs` (`--backend mock|opencode`, default mock), `Cargo.toml`
  (+`ureq 3`, `serde_json`), `scripts/pty_probe.py` (+pty grid parser,
  scenario/resize probes), `research/{opencode-architecture,keymap}.md`.
* Trait changes (justified, documented): `Session.id: usize → String`
  (real ids are `ses_*`); `ToolCall.output: Vec<String>`; new
  `StreamEvent::{ThinkChunk, UpdateTool.output}`; `sessions() → 
  session_summaries() + session_blocks()` (borrowed slices can't cross a
  threaded backend's lock).

## OpenCodeBackend architecture

`connect(config)` → resolve endpoint (env override → `service.json`
health-checked discovery → private `opencode serve` child, never touching
the managed registration) → verify → initial load (sessions + active history
+ commands) → SSE subscriber thread. Snapshots serve memory; REST calls are
single blocking roundtrips with timeouts; worker appends raw envelopes to a
channel drained by `poll_stream`. Auth: none locally, else Basic
`opencode:<password>` from discovery/env (memory only).

## Event mapping table (mapper.rs, pure + tested)

| OpenCode | StreamEvent |
| --- | --- |
| text started/delta/ended | (open)/Chunk/— |
| reasoning started/delta/ended | (open)/ThinkChunk/— |
| tool.input.started (names the tool on 2.0.1) | draft bookkeeping |
| tool.called | Push(Tool Running) — except `shell`, rendered from shell events |
| tool.success/failed | UpdateTool Done/Failed + output; failed also pushes Error |
| shell.started/created | Push(Shell Running) |
| shell.ended lines / shell.exited status | ShellLine… + UpdateTool |
| step/execution.started | Push(Working) (consecutive collapse) |
| step/ended, execution.succeeded | retire spinner (+Error on error finish) |
| step/failed | retire + Error |
| permission.v2.asked | Push(Permission) |
| question.v2.asked | Push(Question) (multi-question supported) |
| prompt/inbox/usage/unknown | ignored (forward-compatible) |

Session mapping: id verbatim, title (slug/Untitled fallback), model label
`agent · provider/model`, cwd from location. Messages: V2 `content[]` and V1
`parts[]` both handled, order-preserving, text/reasoning coalesced.

## Tool/shell/permission/question/cancel/config mappings

* Tools: name + per-tool input summary + Running→Done/Failed + text/file
  output. Shell: command + progressive lines + exit-derived state.
* Permissions: accept→`once`, deny→`reject` via
  `.../permission/{id}/reply` (tracked request ids; history gates resolve
  locally).
* Questions: Asked renders; **no answer route exists on observed servers** —
  answers record a visible Notice instead of pretending (Phase 5).
* Cancel: `Backend::cancel` → `POST .../interrupt` (ctrl-c tier kept).
* Config: env override → discovery → private spawn; project = cwd;
  `Permission.Reply`, `Form` routes, revert/compact/diff reserved.

## Streaming implementation

No changes to the Phase-3 pump: worker appends, `poll_stream` translates +
applies through the shared applier, repaints continue while busy. Verified
word-by-word growth, completion, and footer recovery live.

## Dependencies added

`ureq 3` (sync HTTP/SSE transport, ~10 small transitive deps) +
`serde_json` (tolerant `Value` parsing — no derived schemas, skew-proof).
`cargo tree` inspected; hand-rolled base64 kept (12 lines, RFC-tested).

## Tests

* 54 unit (34 inherited + 20 adapter: mapper/vocabularies, SSE parsing,
  worker delivery, auth vector, tab validation) + 1 ignored live read-only
  test — all passing; fmt clean; zero clippy warnings.
* 18 live pty checks green on mock; adapter verified live separately:
  real sessions/tabs/history/status, submit→stream→complete,
  reasoning+thinking, shell tool end-to-end, gates render.
* Success-path capture used the free tier (~tens of tokens); quota-bearing
  probing avoided; auth copy transient + shredded; 20 ephemeral sessions
  deleted afterwards (verified zero leftovers).

## Known limitations

* Question answering unwired (no route observed); form routes unused.
* History hydration on fresh sessions unproven (old sessions return `[]`).
* DELETE/permission-reply/interrupt proven via CLI/REST ad hoc; adapter
  paths verified except reply (no live gate encountered with adapter).
* Tab strip doesn't auto-reveal appended tabs under overflow.
* Narrow statuslines truncate (Warp-identical policy).
* `session.next.*` shapes source-derived, not live-observed.
* One transient `;43H` prompt fragment seen once under the synthetic pty
  harness (no echo per termios, unreproducible, never under tmux) — logged,
  not attributed to app code.

## Recommended Phase 5

1. Question-answer delivery (observe a live `question.v2` flow or the tool-result channel).
2. Fresh-session history hydration proof + message pagination.
3. Tab auto-reveal on append; context-% via `/context`.
4. Row-level snapshot caching if profiling warrants.
5. Model/agent switching UI against `/api/model`, `/api/agent`.
6. Reconnect/resubscribe torture + busy reconciliation after SSE gaps.
