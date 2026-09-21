# Architecture

Current architecture of OWT, as it actually exists in this repository.
Labels mean exactly what they say: **IMPLEMENTED** is code, tested and
verified; **RESEARCHED** is decision logs and probe evidence with no
production code; **FUTURE** is a direction with nothing built.

```text
                         IMPLEMENTED
Human ── keyboard/mouse/terminal ──► OWT / TUI (src/tui/)
                                          │  snapshots only:
                                          │  summaries, blocks, status
                                          ▼
                                Backend trait (src/backend/mod.rs)
                                MockBackend ◄──► OpenCodeBackend
                                    ┌────────────┬────────────┐
                                    │            │            │
                                    ▼            ▼            ▼
                              HTTP client    SSE worker   Event mapper
                              (ureq,         (live-only,    (tolerant
                               blocking,       backoff        reader)
                               15 s            reconnect)
                               roundtrips)
                                    │            │            │
                                    └────────────┴────────────┘
                                               │
                                                              ▼
                                              OpenCode server (2.0.8 verified)
                                              the agent: models, tools, sessions
                                                              ▲
                                                              │ session start only:
                                              Zero Memory ────┘   frozen owt.memory block
                                              IMPLEMENTED
                                              (src/backend/memory/ + memory_inject.rs)

FUTURE (nothing in this repo):
  Laya ──► intended decision layer beside the model ("what decision
            should be made?"), separate from Zero Memory ("what does
            the system know?"). No code, no prototype, no integration.
RESEARCHED (docs only): Phase 8 intelligence, Phase 9 memory UX.
```

Cleaner view of the adapter's three pillars (all siblings under one
backend, sharing one state):

```text
OpenCodeBackend state (summaries, blocks, busy, gates, discovery caches)
        ▲                 ▲                  ▲
        │ REST            │ SSE              │ translate
   client.rs          events.rs           mapper.rs
```

## Principles

1. **The TUI never sees OpenCode types.** Views render `Backend`
   snapshots (`SessionSummary`, `Block`, `StatusInfo`); gestures call
   trait methods. This is the rule that makes the backend replaceable
   and it has held since Phase 2.
2. **The adapter owns all protocol knowledge.** Version skew, tolerant
   parsing, pagination quirks, and blind-accept footguns (the server
   204-accepts bogus model/agent ids — so the client validates against
   discovery and never sends them) live in `src/backend/opencode/`.
3. **Failures are in-band.** Every fallible path ends as an `Error` or
   `Notice` block in the transcript. Sessions never die silently; busy
   state always reconciles (cancel clears it; reconnect clears it).
4. **Destructive operations confirm.** Revert shows the real staged
   files and states both destructive axes before digit-confirmation;
   file-writing slash commands (`init` and unknowns) gate the same way.
   Nothing auto-stages, auto-commits, or auto-retries mutations —
   especially across reconnects.
5. **Memory is small on purpose.** The Phase 2B minimality review
   killed scores, SQLite/FTS5, and six-component designs. What exists:
   API + JSONL store + budget + one frozen injection. Intelligence stays
   a research verdict (deterministic, user-gated, no new deps).
6. **Zero Memory and Laya are separate concepts.** Knowledge vs
   decision. Even as a future direction, they must not be merged.
7. **The model reasons; OWT presents.** The selected OpenCode model
   remains the primary reasoning/generation engine. OWT does not
   second-guess it — it frames the interaction and preserves context.

## Component map (all IMPLEMENTED unless noted)

| Component | Path | Role |
| --- | --- | --- |
| Trait + models | `src/backend/mod.rs` | `Backend` contract, `Block` vocabulary, `Blocker`, snapshots |
| Stream applier | `src/backend/stream.rs` | Shared `StreamEvent` → blocks logic for every backend |
| Mock | `src/backend/mock.rs` | Scripted deterministic backend; dev + tests + PTY |
| HTTP client | `src/backend/opencode/client.rs` | Thin verified wrappers over OpenCode REST routes |
| SSE worker | `src/backend/opencode/events.rs` | Subscriber thread, backoff reconnect, synthetic gap signals |
| Mapper | `src/backend/opencode/mapper.rs` | Wire envelopes → blocks/events; drops nothing meaningful |
| Adapter state | `src/backend/opencode/mod.rs` | Sessions, gates, history, 7C ops, injection hooks |
| Injection | `src/backend/opencode/memory_inject.rs` | Capability probe + frozen `owt.memory` PUT at session start |
| Memory engine | `src/backend/memory/` | `api/store/record/budget/command/key/secret/tombstone`; JSONL in `$XDG_DATA_HOME/owt/` + per-project `.owt/` |
| Session UI | `src/tui/session.rs` | Composition root: tabs, transcript, menu, prompt, statusline, keys |
| Transcript | `src/tui/transcript.rs` | Block → Warp-styled rows, markdown-lite, diff rendering |
| Prompt/menus/status | `src/tui/{prompt,menus,statusline}.rs` | Input, slash-command menu from discovery, footer |
| Widgets | `src/tui/widgets/` | Promoted Warp snapshots (compilable copies; see `NOTICE.md`) |
| Reference | `warp-tui/` | Pristine frozen Warp snapshots — never modified |
| Harness | `scripts/pty_probe.py` | Drives the real binary under a pty; exact screen assertions |

Data flow, concretely: keypress → `SessionAction` → trait method →
adapter REST call and/or local state → SSE events → `ingest` → mapper →
`apply_event` → blocks → next frame renders snapshots. Memory flows one
way, once per session: engine → budget → PUT `owt.memory` → frozen.

## What this project is NOT

- **Not a fork of Warp.** Warp is a pinned reference (`1bf1c6a`);
  `~/warp` is untouched; only permitted framework code was reused with
  attribution (`NOTICE.md`, `research/licensing.md`).
- **Not a replacement for OpenCode.** OpenCode is the agent. OWT is the
  interface around it. If the server goes away, OWT has nothing to say.
- **Not a new foundation model.** The models come from OpenCode
  providers (much of the probing used free/low-cost Zen models). OWT
  has no training, no weights, no eval harness.
- **Not an inventor of memory systems.** The docs cite existing work
  (Mem0, Letta, holographic and Hindsight approaches, among others in
  `research/memory/`). The contribution, if any, is a small honest
  local-first implementation with its evidence attached.
- **Not Laya-integrated.** Laya appears nowhere in code. Any sentence
  claiming otherwise is wrong; correct it on sight.

## Where to start

1. `README.md` — what this is and how to run it.
2. `PROJECT_ORIGIN.md` — how it grew here.
3. This file — how it fits together.
4. `phases.md` + `AGENTS.md` — roadmap records and working rules (note:
   the phases status table can lag the `PHASE*_COMPLETE.md` reports;
   trust the reports for what was actually verified).
5. Source, in this order: `src/backend/mod.rs` → `src/backend/mock.rs`
   → `src/backend/opencode/` → `src/backend/memory/` →
   `src/tui/session.rs` → `src/tui/transcript.rs`.
6. Tests: unit tests beside the code; `scripts/pty_probe.py` end to end;
   `research/` for why things are the way they are.
