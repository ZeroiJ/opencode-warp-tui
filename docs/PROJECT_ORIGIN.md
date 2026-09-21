# Project Origin and Evolution

This is the story of how a weekend-sized idea — a nicer TUI for OpenCode —
turned into a multi-phase agent-systems experiment. Every stage below is
traceable in the repository: git history, `phases.md`, the `research/`
decision logs, and the `PHASE*_COMPLETE.md` reports.

## Stage 1 — A simple idea

*Build a better/custom Warp-inspired TUI around OpenCode.*

That was the whole pitch. Warp's terminal UI had interaction and
presentation patterns worth learning from (block transcript, gates,
menus, statusline), and OpenCode needed a client. Nothing about memory,
nothing about decision layers, no broader architecture. Just the TUI.

## Stage 2 — Reverse engineering and research

Before writing anything ambitious, the project did the homework: the
Warp repository (pinned at commit `1bf1c6a`, September 2026) was mapped
(`research/warp-tui-map.md`, `research/dependency-map.md`), its
component dependencies tabulated, and its licensing split documented
(MIT `warpui`/`warpui_core` vs AGPL-3.0-only everything else —
`research/licensing.md`). Seven presentation-only files were snapshotted
into `warp-tui/` as a frozen reference.

Two decisions from this stage still govern the project: Warp is a
**reference, never a backend** (the checkout at `~/warp` is verified
clean after investigation work to this day), and only what's legally and
technically appropriate gets reused.

## Stage 3 — Standalone OWT

Rather than forking or patching Warp, the project became its own Rust
crate: `opencode-warp-tui`, binary `owt` (git: `Phase 1-4`). The
foundation was built on local abstractions — a generic `Backend` trait,
a scripted `MockBackend`, the polled-`StreamEvent` pump, multiline
input, tabs, transcript rendering over genuine `warpui_core` elements —
all validated against mock data before any network code existed. The
TUI/backend separation from this stage is still the load-bearing wall:
the TUI has never been allowed to depend on OpenCode types directly.

## Stage 4 — OpenCode integration

With the frontend standing on its own, the adapter was built: an
`OpenCodeBackend` implementing the same trait, speaking to a live
OpenCode server over HTTP (`ureq`) and SSE (`/api/event`). This is where
the project learned how much of the "simple TUI" idea was actually
protocol work — version-skewed wire shapes, tolerant readers, session
management, and the realization (later, in Phase 7A research) that some
documented routes simply don't exist on the live server.

## Stage 5 — Agent interaction

Sessions, streaming, question and permission gates, history hydration,
reconnect robustness, cancellation, errors. The unglamorous middle that
makes a wrapper feel like a tool: digit-key gate answers, cursor-paged
oldest-first history (the server's default newest-first order renders
transcripts inverted if you're careless), synthetic
`connection.lost`/`restored` signals so a dropped stream can't spin the
statusline forever, and a rule that every failure is a visible in-band
error — the session never dies silently. The headline discovery: on
OpenCode 2.0.8 the `question` tool never emits `question.v2.asked`; it
materializes as a `form.created` event answered via a form-reply route —
found by live probe, not by reading docs.

## Stage 6 — Advanced operations

Compact, diff, revert, fork, model switching, agent switching, and
server-side slash-command execution. Each one required its own live
probe before implementation, and each probe taught something: the server
204-accepts bogus model/agent ids into dead turns (so the client must
validate against discovery and never send them); revert is two different
destructive axes (stage restores files immediately, commit deletes
messages irreversibly — hence mandatory explicit confirmation showing
the real affected files); fork children inherit history but not
instruction entries or revert boundaries. All of it rides the existing
seams: the same gates, the same transcript blocks, zero TUI redesign.

## Stage 7 — Memory: "What if the agent could retain useful knowledge?"

At some point the project started asking a larger question: *what if
the agent could retain useful knowledge across sessions?* That became
the Zero Memory architecture — "what does the system know?"

It was researched the same way everything else was: an OpenCode-
compatible memory architecture study, a minimality review that killed
the ambitious six-component candidate (no SQLite, no FTS5, no scores),
and a storage/schema/injection design that survived into code. What got
built is deliberately small: a `MemoryApi` over a file-backed JSONL
store, fact/preference records, user/project scopes, a token-budgeted
context builder, secret refusal, `/memory` commands, and exactly one
frozen `owt.memory` block injected at session start. Memory
intelligence (extraction, hybrid retrieval, decay) was researched and
then deliberately **not** built — its verdict, recorded in the Phase 8
docs, is deterministic user-gated intelligence with no new
dependencies, waiting for authorization that may or may not come.

## Stage 8 — A decision layer: "What decision should be made?"

The current architectural idea, and it is only an idea: Zero Memory and
a prospective layer called **Laya** are not the same thing and must
never be merged into one blob:

- Zero Memory: *"What does the system know?"*
- Laya: *"What decision should be made?"*
- The selected OpenCode model: *"How should the task be reasoned about
  and communicated?"*
- OWT: *"How does the human interact with the system?"*

Laya would sit alongside the model as a separate decision-making layer.
There is no Laya code in this repository, no prototype, no integration —
saying otherwise would be dishonest. It is a research candidate for a
future phase, nothing more, and it stays out of the memory system by
design.

## The model experiment

A major motivation throughout — and the reason the probes kept going
deeper than strictly necessary — has been experimentation with the
free and low-cost models available through OpenCode (things like
`mimo-v2.5-free` and `muse-spark-1.3` on the Zen provider, plus whatever
else `GET /api/model` offered that week). The author deliberately tested
how much sophistication these models could handle: real implementation
work, multi-step probes, increasingly complex workflows.

There are already research papers, commercial systems, and open-source
projects demonstrating that sophisticated agent, memory, and decision
architectures are possible. The goal here isn't to pretend otherwise.
Part of this project exists because the author wanted to find out
first-hand how far these models and systems could actually be pushed —
not as a benchmark, but as a builder's question: can the idea survive
contact with a real implementation?

Some of the motivation is simply curiosity and self-satisfaction. The
pieces were built by hand to understand the systems underneath them:
the TUI framework, the event protocol, the storage format, the
injection seam. That is also why the methodology is so stubborn about
verification — live probes, throwaway sessions, deleted afterwards,
evidence in `research/` — rather than claims. If an idea couldn't
survive the implementation, it stayed a research doc.

## Where that leaves us

A personal TUI experiment grew, layer by layer, into a working
agent-system workbench with its own memory foundation and an open
question about decision layers. Nothing here claims to replace OpenCode,
invent memory systems, or ship artificial general anything. It is an
honest record of pushing modest models and systems as far as they would
go — and writing down exactly where they stopped.
