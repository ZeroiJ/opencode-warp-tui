# OWT — OpenCode Warp TUI

OWT (`owt`) is a standalone, Warp-inspired terminal UI for driving
[OpenCode](https://github.com/sst/opencode). It is an **interface and
integration layer**: OpenCode remains the agent/backend that does the
actual work; OWT is the human-facing surface around it — sessions,
transcript, gates, history, and a growing set of agent operations.

> **Status: Phases 6B, 7B, and 7C implemented and verified.**
> The crate builds to a working TUI (`cargo run`, binary `owt`) with a
> live `OpenCodeBackend` behind the generic `Backend` trait (mock is the
> default), a local persistent memory engine (`/memory` commands, JSONL
> storage, single frozen `owt.memory` injection at session start), and
> advanced operations (compact, diff, revert with confirmation, fork,
> validated model/agent switching, server slash-command execution).
> Test suite: 195 passed / 1 ignored; PTY harness 18/18 green.
> Phase 8 (memory intelligence) and Phase 9 (memory UX) are
> **research-only** so far. See `phases.md` (note: its status table can
> lag the `PHASE*_COMPLETE.md` reports) and `ARCHITECTURE.md`.

## Why does this exist?

The original goal was small: build a better/custom Warp-style TUI around
OpenCode instead of living in whatever client happened to be available.
Understanding Warp's TUI took real reverse-engineering, connecting to
OpenCode's HTTP/SSE API took real adapter work, and each layer (sessions,
streaming, gates, history, memory) turned out to be its own project.
The scope grew organically from a TUI experiment into a broader
agent-system experiment. There is a section about that below.

## What does OWT currently do?

- **Sessions as tabs**: create, switch, and follow multiple OpenCode
  sessions (`ctrl-p` / `ctrl-n` / `ctrl-o`).
- **Streaming transcript**: assistant text/thinking, tool calls with live
  state, shell output, file diffs, plans, errors — pumped as polled
  `StreamEvent`s, no TUI threads.
- **Gates**: permission requests (allow-once / reject) and agent questions
  answered with digit keys. Question answers are delivered over OpenCode's
  verified form-reply route; every failure path is a visible in-band
  error, never a dead session.
- **History + reconnect**: cursor-paginated, oldest-first hydration; the
  SSE worker emits synthetic `connection.lost` / `connection.restored`
  signals so a dropped stream clears stale busy state and surfaces one
  notice per gap instead of spinning forever.
- **Advanced operations** (typed as `/`-commands): `compact`, `diff`,
  `revert` (stage → show affected files → explicit digit confirmation →
  commit or abandon), `fork` (full or before-a-message), `model` /
  `agent` switching (validated client-side against server discovery —
  OpenCode 204-accepts bogus values, so invalid ids are never sent), and
  server slash commands (file-writing commands like `init` always
  confirm first).
- **Memory**: a local persistent knowledge store with `/memory remember /
  update / forget / pin / list / show`, secret refusal, and one frozen
  `owt.memory` context block injected at session start. See "What is
  Zero Memory?" below.

## How does it interact with OpenCode?

OWT talks to a running OpenCode server (verified against 2.0.8) over
HTTP (`ureq`, blocking, 15 s timeouts) and a server-sent-events stream:

- REST for sessions, prompts, interrupts, permissions, question-form
  replies, compact/diff/revert/fork/model/agent/command routes, and
  paged message history.
- SSE (`/api/event`) for live deltas, tool lifecycle, forms, compaction
  and revert events. The stream is live-only (no replay); after a gap,
  state is re-based from REST.
- All OpenCode knowledge lives behind the adapter boundary
  (`src/backend/opencode/`); the TUI only ever sees the generic
  `Backend` trait, so the backend stays replaceable.

## What is Zero Memory?

Zero Memory is this project's name for its persistent knowledge/context
system — the answer to "what does the system know?" (In code it is the
*memory engine*: `src/backend/memory/` plus the `owt.memory` instruction
entries written into OpenCode sessions.)

Today it is a deliberately small foundation: a `MemoryApi` over a
file-backed JSONL store (`$XDG_DATA_HOME/owt/` + per-project `.owt/`),
fact/preference records, user/project scopes, a token-budgeted context
builder, and one frozen injection per session. No embeddings, no vector
search, no automatic extraction — those belong to the researched-but-
unimplemented Phase 8 (memory intelligence), whose verdict is
deterministic, user-gated intelligence with no new dependencies.

## What is Laya intended to become?

Laya is the name for a **possible future decision-making layer**, kept
strictly separate from memory:

- Zero Memory: *"What does the system know?"*
- Laya: *"What decision should be made?"*
- The selected OpenCode model: *"How should the task be reasoned about
  and communicated?"*
- OWT: *"How does the human interact with the system?"*

To be completely clear: **Laya does not exist in this repository.** There
is no Laya code, no Laya integration, no Laya prototype — only the
architectural direction described here. Do not confuse it with Zero
Memory; they are separate concepts and must stay that way.

## Implemented vs experimental vs planned

| Area | State |
| --- | --- |
| TUI (tabs, transcript, prompt, menus, statusline) | Implemented |
| MockBackend (scripted, deterministic) | Implemented |
| OpenCode adapter (sessions, streaming, gates, history, reconnect, cancel) | Implemented, live-verified vs 2.0.8 |
| Advanced ops (compact/diff/revert/fork/switches/commands) | Implemented, live-verified |
| Memory engine + session-start injection | Implemented |
| Phase 8 memory intelligence (extraction, hybrid retrieval, decay…) | Research only (`research/memory/phase8-*.md`) |
| Phase 9 TUI memory UX | Planned, not designed in code |
| Phase 10 production hardening | Planned |
| Laya decision layer | Future direction; nothing implemented |

## How the pieces relate

```text
Human
  │  keyboard / mouse / terminal
  ▼
OWT / TUI (`src/tui/`) — renders `Backend` snapshots, sends gestures back
  │  generic Backend trait (`src/backend/mod.rs`)
  ▼
OpenCodeBackend (`src/backend/opencode/`) — HTTP client, SSE worker,
event mapper, per-session state, memory-injection hook
  │  HTTP + SSE
  ▼
OpenCode server (2.0.8 verified) — the agent: models, tools, sessions
  │
  ├── Zero Memory feeds in at session start (frozen `owt.memory` block)
  └── (future) Laya would sit beside the model as a decision layer
```

MockBackend implements the same trait for deterministic development and
tests; the TUI cannot tell the backends apart.

## How to run it

```sh
cargo run                                  # mock backend (default)
cargo run -- --backend opencode            # live OpenCode server
```

The live backend discovers the managed OpenCode service registration or
spawns its own server (see `src/backend/opencode/config.rs`). Mock tips:
type text, `enter` to submit (`ctrl-j` for newline), `/` for the command
menu, `/demo b` for a streaming answer, `?` for shortcuts, `ctrl-t` to
simulate activity, `ctrl-o` for a new session, wheel / `pgup/pgdn` to
scroll, `ctrl-c` (×2) to exit. `mise x rust -- cargo test` runs the
suite; `python3 scripts/pty_probe.py ./target/debug/owt` drives the real
binary under a pty.

## How did this get so big?

It wasn't supposed to.

The original idea was to build a custom TUI around OpenCode. Then we
needed to understand the TUI architecture. Then we needed real OpenCode
integration. Then sessions. Then streaming. Then questions. Then
permissions. Then reconnects. Then advanced operations.

Eventually the question changed from "Can I make OpenCode look better?"
to "What would an agent system built around OpenCode look like if I
controlled the surrounding architecture?"

That is where Zero Memory entered the picture.

And now we're investigating whether a separate decision layer, Laya, can
sit alongside the selected OpenCode model.

So yes, the scope escaped containment.

## Current development status

Phases 1–5, 6B, 7B, 7C are implemented and verified (see
`PHASE6B_COMPLETE.md`, `PHASE7B_COMPLETE.md`, `PHASE7C_COMPLETE.md`).
Phase discipline is strict: no phase starts without explicit
authorization, and `~/warp` (the reference checkout) is never modified.
The canonical rules live in `AGENTS.md`.

## Where should a contributor start reading?

1. This file, then `PROJECT_ORIGIN.md`, then `ARCHITECTURE.md`.
2. `phases.md` (roadmap + per-phase records) and `AGENTS.md` (working rules).
3. `src/backend/mod.rs` (the trait — the single most important file),
   `src/backend/mock.rs`, `src/backend/opencode/`, `src/backend/memory/`.
4. `src/tui/session.rs` (composition root of the UI), `src/tui/transcript.rs`.
5. `research/` for decisions and live-probe evidence; `scripts/pty_probe.py`
   for end-to-end verification.

License: AGPL-3.0-only overall (see `NOTICE.md`). The TUI framework
(`warpui_core`) is MIT-licensed and pinned; `warp-tui/` holds pristine
reference snapshots that must not be modified.
