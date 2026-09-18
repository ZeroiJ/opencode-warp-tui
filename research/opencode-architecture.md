# OpenCode architecture (for the adapter)

Examined 2026-09-18. All findings verified against the installed server or
the published contract unless marked otherwise. Factual notes, not advice.

## Version under test

* Binary: `opencode` 1.18.31 (mise), anomalyco lineage (single Bun binary).
* Docs/spec fetched: `https://opencode.ai/v2/openapi.json` (OpenAPI 3.1.0,
  113 paths, 242 schemas, `info.version 0.0.1`) — describes the NEW
  experimental HttpApi and **lags or leads the installed server in places**;
  live behavior wins below. The TS client's `Service.ensure` checks
  `version.startsWith("2.")`, so 1.18.x predates the V2 line: expect skew.
* Source (read-only, exact paths): `sst/opencode`
  `packages/schema/src/{event-manifest,event,session-event,question,permission}.ts`,
  `packages/opencode/src/{bus/global.ts,session/}`.

## Server architecture

* `opencode serve --port N --hostname 127.0.0.1` → headless HTTP server
  (Bun). Serves the web UI (SPA fallback for unknown routes) + JSON API.
* No auth by default (`OPENCODE_SERVER_PASSWORD` unset → "unsecured"
  warning). With a password: **HTTP Basic, user `opencode`** (verified live;
  Bearer / x-api-key / query all 401).
* Errors: JSON envelope `{"_tag": ..., "message"?, "kind"?}`, e.g.
  `InvalidRequestError`/`Payload`, `UnauthorizedError`, `UnknownError{name,data{message,ref}}`.
* State: SQLite at `~/.local/share/opencode/opencode.db` (via `opencode db`).
  A fresh DB migrates cleanly; the user's existing DB lacked `session_input`
  (V2 prompt path needs it) — probe servers used `XDG_DATA_HOME` isolation.
* Service registration: `~/.local/state/opencode/service.json`
  `{id, version, url, pid, password?}` — the discovery mechanism
  (`Service.discover`). A private adapter child server must NOT claim it.
* Locations: most routes are location-scoped (`?location[directory]=...`
  deepObject query). Sessions carry `location{directory}` + `subpath`.

## REST API (verified live except where noted)

| Operation | Verified |
| --- | --- |
| `GET /api/session` → `{data: Session.Info[]}` | live |
| `POST /api/session` `{title?}` → created info | live |
| `GET /api/session/{id}` | live |
| `DELETE /api/session/{id}` | spec-only (live attempt hit SPA fallback; use `opencode session delete` meanwhile) |
| `GET /api/session/{id}/message?limit=` → `{data: Message.Info[], cursor{previous,next}}` | live (empty on old sessions; fresh-session hydration unverified) |
| `POST /api/session/{id}/prompt` body **`{"prompt":{"text",...}}`** (spec shows flat `{text}` — live 1.18.31 wants the wrap) → `{data:{admittedSeq, id: msg_*, sessionID, prompt, delivery}}` | live |
| `POST /api/session/{id}/interrupt` (empty body) | spec-only |
| `GET /api/session/active` → `{data: {sesID: SessionActive}}` | spec-only |
| `POST /api/session` fork/agent/model/move, revert, compact, context, diff, inbox, generate, wait, background, synthetic, shell, environment, view | spec-only (not needed for Phase 4) |
| `POST /api/session/{id}/permission/{reqID}/reply` `{decision: once\|always\|reject, message?}` | spec-only (schema-verified) |
| Forms (`.../form`, `.../form/{id}/reply`, DELETE cancel) | spec-only |
| No question-reply route exists in the spec | fact (gap, see Questions) |
| `/api/shell*` routes | spec-only (live attempts inconclusive) |
| `GET /api/agent`, `/api/model`, `/api/command`, `/api/skill` | spec-only |
| `GET /api/event` → SSE `text/event-stream` | live |

## Event stream (SSE, verified live + source)

* Envelope (live): `{"id":"evt_*","type":"...","durable"?{aggregateID,seq,version},"location"?{directory},"data":{...}}`, plus `: heartbeat` lines. Live-only, no replay; late subscribers get `server.connected`.
* Observed live: `server.connected`, `session.created`, `session.next.prompt.admitted`, `session.next.prompted`, `session.next.step.started`, `session.next.step.failed`, `plugin.added`, `catalog.updated`, `reference.updated`, `integration.updated`.
* Full vocabulary (`packages/schema/src/session-event.ts`):
  text started/delta/ended, reasoning started/delta/ended, tool
  called/input.started/input.delta/input.ended/progress/success/failed,
  shell started/ended, step started/ended/failed, prompt.admitted/prompted,
  compaction started/delta/ended, context.updated, retried, revert
  staged/committed/cleared, synthetic, agent/model switched, moved.
* Key payloads (source): text.delta `{assistantMessageID, textID, delta}`;
  text.ended adds full `text` (replayable boundary); reasoning same shape;
  tool.called `{callID, tool(name), input{}, provider{executed}}`;
  tool.success/failed carry `content: Tool.Content[]` (text/file) or
  `error: StructuredError`; shell.started `{messageID, callID, command}`,
  shell.ended `{callID, output}`; step.ended `{finish, cost, tokens{...}}`.
* Status: `session.idle`, `session.status` (`session-status-event.ts`).
* Deltas are live-only fragments; `*.ended` events carry full values.

## Sessions

`Session.Info{id: ses_*, parentID?, projectID, agent, model{id,providerID,variant}, cost, tokens{input,output,reasoning,cache{read,write}}, outcome?, time{created,updated,...}, title, subpath?, permissions: Ruleset, location}`.
Event variant adds `slug` ("stellar-cactus"), `version`, `directory`, `path`.
Children link via `parentID` (subagents). Titles auto-generated ("New session - …").

## Messages

* V2 REST: `User{id: msg_*, time, text, files?, agents?, skills?}`,
  `Assistant{id, time{created,streamed?,completed?}, agent, model,
  content: (Text{text} | Reasoning{text} | Tool{id,name,executed,state})[],
  finish: stop|length|tool-calls|…|error, error?, retry?}`,
  plus Shell `{shellID: sh_*, command, status: running|exited|timeout|killed,
  exit, output{output,cursor,...}}`, Compaction, Idle, System, Synthetic…
* V1 export (`opencode export`, verified on a real session): messages with
  `info{role,time,tools}` + `parts[]`: `text`, `reasoning`, `tool{tool(name),
  callID, state{status,input{},output:"..."}}`, `step-start/step-finish`,
  `file`. Tool input/output are plain JSON/string — ideal history hydration.

## Permissions

* Model (docs `/docs/permissions`): keyed by tool name (`bash`, `edit`,
  `question`, …) + guards; rules resolve allow/ask/deny; UI outcomes
  **once / always / reject**; `Permission.Reply = once|always|reject`.
* Wire: `permission.v2.asked` / `permission.v2.replied`
  (`packages/schema/src/permission.ts`); REST request
  `{id: per_*, sessionID, action, resources[], save?, metadata?, source?,
  message?}`; reply `POST .../permission/{reqID}/reply`.
* Adapter maps accept→`once`, deny→`reject` (`always` reserved).

## Questions

* `QuestionV2.Request{id: que_*, sessionID, questions[{header(≤30ch),
  question, options[{label(1–5 words), description}], multiple?, custom?}],
  tool?}`; answers are per-question label arrays
  (`Reply{answers: Answer[]}`); events asked/replied/rejected.
* **Gap**: no question-reply HTTP route in the 1.18.31-visible spec; the
  `question` tool blocks until answered through an undocumented channel
  (possibly the tool-result path or forms). Adapter surfaces Asked in the UI
  and records the gap; answer delivery is Phase-5 work after observing a live
  question event.

## Tools & shell

* Tool identity = `tool.called` name + `callID`; input streams
  (started/delta/ended) then `tool.success` (content[]) / `tool.failed`
  (error + content). History shows input objects + string output.
* Detail summarization is adapter-side (per-tool input keys: command, file,
  pattern, query…).
* Session shell events carry full command + output; `/api/shell*` routes
  deferred.

## Cancellation

`POST /api/session/{id}/interrupt` (empty body). Adapter keeps the active
session id; `Backend::cancel` maps to one interrupt call (fire-and-forget;
server settles with `step.failed`/`session.idle` events).

## Config & auth

* Server URL: explicit override env (`OPENCODE_SERVER_URL`, adapter-defined)
  → live `service.json` discovery (version/pid/health-checked) → spawn
  `opencode serve --port <free> --hostname 127.0.0.1` as a private child
  (parsed from its `listening on …` line; killed on drop).
* Auth: none locally by default; Basic `opencode:<password>` when the server
  sets one (from discovery record or `OPENCODE_SERVER_PASSWORD`).
* Project: adapter works in the user's cwd project (server inherits env);
  sessions record their own `location.directory`.
* Secrets: never in source; password only in memory from discovery/env.

## Uncertainties / assumptions

* Published spec (V2) vs installed server (1.18.31) skew is real (prompt
  body shape); adapter targets installed behavior, verified live per call.
* DELETE session, interrupt, permission/form replies, message history on
  fresh sessions: spec-shaped, pending live proof in Phase 5.
* Streaming success path (text deltas for a real answer) unobserved — no
  valid provider credentials in the sandbox; shapes come from source.
* `opencode run` in the probe dir was hijacked by the user's OhMyOpenCode
  orchestration config — headless runs inherit global config; adapter
  servers likewise inherit user config (acceptable: it IS their setup).
