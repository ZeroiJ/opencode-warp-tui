# Memory Research Phase 1 — Research Plan

> Status: 🔬 IN PROGRESS (Phase: "Memory Research Phase 1 — OpenCode-Compatible
> Long-Term Memory Architecture")
> Date: 2026-09-18
> Type: **Research only.** Nothing in this phase is implemented; no Cargo
> dependencies are added; no OpenCode files are modified; no database or
> embeddings are created. See `phases.md` "🚫 DO NOT START YET".
> Evidence labeling: **[VERIFIED]** primary source / live probe ·
> **[OBSERVED]** seen but not independently confirmed · **[INFERRED]**
> analytical conclusion derived from verified facts.

---

## 1. Objective

Determine how this project (`opencode-warp-tui`, a Warp-style Rust TUI that
talks to OpenCode through a generic `Backend` trait) can be given a durable
long-term memory subsystem that:

1. **Works cleanly with OpenCode** — no OpenCode fork, no invasive
   modification, survives OpenCode upgrades.
2. **Is local-first** — no mandatory cloud dependency; private by default.
3. **Supports the TUI's real boundaries** — the memory engine must attach at
   the adapter layer (or as a sidecar), never inside the TUI rendering code,
   per `AGENTS.md` phase discipline.
4. **Covers the full memory lifecycle** — capture, extraction, storage,
   retrieval, updates, contradictions, deletion, consolidation.
5. **Respects scope** — cross-session, per-project, per-user.
6. **Avoids context pollution and excessive token use** — injection must be
   bounded and visible.
7. **Produces a documented architecture decision** for Memory Research
   Phase 2 (`architecture.md` §36 recommendation; Tier 1/2/3 designs §37).

## 2. Constraints (from `AGENTS.md` and `phases.md`)

| Constraint | Consequence for this research |
|---|---|
| Research only; no implementation | Deliverables are documents, not code |
| Do not modify OpenCode | Adapter/plugin-or-sidecar surface only; "experimental" APIs flagged |
| Do not fork OpenCode | No forked code paths; behavior must ride on stable/HTTP surface |
| Keep OpenCode integration isolated from TUI | Memory engine attaches to adapter boundary, not to renderer |
| Do not add Cargo dependencies | No `sqlite`, `hnsw`, `embedding` crates this phase |
| Do not modify `warp-tui/src/` (pristine snapshots) | Only reads; research does not touch it |
| Crate is AGPL-3.0-only overall | Third-party code reuse must respect licenses (MIT/Apache/AGPL) |
| Do not alter OpenCode's SQLite schema | Memory storage is ours, separate |
| Warp checkout `~/warp` stays untouched and clean | Verify `git status --porcelain=v1` empty after research |

## 3. Research questions (15 categories, from `phases.md`)

1. **OpenCode compatibility** — which integration surfaces exist today
   (HTTP API, plugin hooks, events, attachments, instructions)?
   What is stable vs experimental?
2. **Memory lifecycle** — capture → extract → store → retrieve → use →
   update → consolidate → forget. Who triggers each step; async or sync?
3. **Memory extraction** — LLM-driven vs heuristic vs agent-self-managed;
   ADD-only vs ADD/UPDATE/DELETE; dedup.
4. **Memory types** — facts, preferences, decisions, experiences, entities,
   procedural/tool knowledge, profiles.
5. **Memory scope** — user/global, project/repo, session; how scopes are
   isolated and queried.
6. **Retrieval** — semantic, keyword/FTS, hybrid, graph, temporal; fusion
   (RRF); reranking; token-budgeted injection.
7. **Hybrid search** — how keyword + vector + entity + time are combined.
8. **Storage** — files, SQLite, embedded vector, external DB; migration;
   durability; audit trail.
9. **Contradiction handling** — overwrite vs refine vs versioned; evidence
   preservation.
10. **Consolidation** — background/sleep-time vs session-end vs on-demand;
   dedup; decay; trust scoring.
11. **Token/context management** — budgets, progressive loading, prefix-cache
    stability, compaction hooks.
12. **Privacy and secrets** — redaction, local-only processing, opt-in
    cloud, telemetry avoidance.
13. **User control** — explicit tools/commands, list/inspect/delete, opt-out.
14. **Version compatibility** — behavior across OpenCode upgrades; which
    hooks/APIs break.
15. **Adapter architecture** — where memory logic lives relative to
    `src/backend/`; trait/interface implications; sidecar vs in-process.

## 4. Investigation targets

| Target | Why | Evidence |
|---|---|---|
| Hermes Agent memory stack | Mature agent memory: builtin curated store + pluggable providers; OpenCode-adjacent architecture; MIT | `hermes_memory_{manager,provider,tool_store}.py` read in full (NousResearch/hermes-agent) |
| Hindsight (vectorize-io) | retain/recall/reflect; TEMPR fused retrieval; official OpenCode plugin | Subagent deep dive; official docs; npm plugin verified |
| Mem0 | V3 ADD-only extraction; factory design; scopes; official OpenCode plugin | Subagent deep dive on `mem0/memory/main.py`; docs |
| Letta (MemGPT) | Three-tier memory (core/archival/recall); blocks; sleep-time; server model | Subagent deep dive; docs; arXiv:2504.13171 |
| OpenViking (Volcengine) | `viking://` FS; L0/L1/L2; session commit; official OpenCode plugin | Subagent deep dive; docs.openviking.ai |
| Holographic (Hermes provider) | SQLite+FTS5 + HRR algebraic retrieval; trust scoring | Subagent summary + downloaded `store/retrieval/holographic/__init__` source |
| OpenCode memory projects | 25+ community plugins; de facto standards; hook fragility | Subagent survey; npm/GitHub checks |
| OpenCode itself | Live 2.0.8 HTTP API; instructions entries; events; plugin hooks | Live probes against `http://127.0.0.1:49374`; OpenAPI saved to `/tmp/opencode/openapi-2.0.8.json` |

## 5. Method

1. **Primary sources first** — repos, docs, papers, source files; no blog
   hearsay without the underlying source.
2. **Live verification for OpenCode** — probe the running 2.0.8 service
   (read-only, ephemeral sessions cleaned up). Key verified facts recorded
   in `research/opencode-architecture.md` (Phase-4 file) and summarized in
   each memory doc where relevant.
3. **Subagent deep dives** for the six non-Hermes frameworks (fetched source
   and official docs; notes staged in `/tmp/opencode/subagent-notes/`).
4. **Hermes read in full** from source (manager/provider/tool store).
5. **Evidence labeling** — every claim tagged **[VERIFIED]** /
   **[OBSERVED]** / **[INFERRED]**.
6. **Synthesis** — comparison matrix (§9–§22), constraints mapping (§23–§33),
   architecture (§34–§37), final report structure (§42).
7. **Stop gate** — after `research-report.md` is written, STOP. No
   implementation, no next phase.

## 6. Deliverables and master outline (§1–§42)

The unified research report is split across files; section numbers continue
across files so citations can be exact (`architecture.md §34`).

| § | Section | File |
|---|---|---|
| 1 | Research plan & methodology | `research/memory/research-plan.md` (this file) |
| 2 | Hermes Agent memory | `research/memory/hermes.md` |
| 3 | Hindsight | `research/memory/hindsight.md` |
| 4 | Mem0 | `research/memory/mem0.md` |
| 5 | Letta | `research/memory/letta.md` |
| 6 | OpenViking | `research/memory/openviking.md` |
| 7 | Holographic (Hermes provider) | `research/memory/holographic.md` |
| 8 | OpenCode memory projects | `research/memory/opencode-projects.md` |
| 9–22 | Comparative analysis: capability matrix and dimensions | `research/memory/comparison.md` |
| 23–33 | Constraints mapping for **this** project + synthesis | `research/memory/comparison.md` |
| 34 | Memory system architecture (**Mermaid diagram**) | `research/memory/architecture.md` |
| 35 | Component specifications | `research/memory/architecture.md` |
| 36 | Recommendation — **Options A/B/C** | `research/memory/architecture.md` |
| 37 | **Tier 1/2/3** designs | `research/memory/architecture.md` |
| 38–41 | Findings, evidence, risks, next-phase scope | `research/memory/research-report.md` |
| 42 | **Final report structure** | `research/memory/research-report.md` |

## 7. OpenCode integration surface verified this phase

Evidence base (see `research/opencode-architecture.md` for the Phase-4
writeup; extended with 2.0.8 findings):

- **[VERIFIED]** API-managed session instruction entries —
  `PUT /api/experimental/session/{id}/instructions/entries/{key}`
  (body `{"value": ...}`), listed in the OpenAPI spec and **round-tripped
  live** with an ephemeral session (zero leftovers). Docs describe it as
  announcing "at next step boundary". `experimental` prefix ⇒ unstable
  across upgrades.
- **[VERIFIED]** No built-in memory — `anomalyco/opencode` issues #24030,
  #20322.
- **[VERIFIED]** Plugin hooks in current docs: `event`,
  `tool.execute.before/after`, `shell.env`, custom `tool: {...}`,
  `experimental.session.compacting`. Plugins load from
  `~/.config/opencode/plugins/` and `.opencode/plugins/`.
- **[OBSERVED]** `experimental.chat.system.transform` is not in current
  official docs, yet many community plugins rely on it — treat as
  unverified/at-risk.
- **[VERIFIED]** Session/message/project/token/cost schemas; active-context
  endpoint; `synthetic` + `generate` endpoints; VCS diff endpoint;
  session delete (204).

## 8. Open questions consciously deferred

- Exact embedding strategy (sidecar process, model, dims) — Phase 2/5.
- Whether to ever ship an OpenCode plugin vs stay adapter-side only — Phase 2.
- Storage engine choice (SQLite pure vs SQLite+sqlite-vec) — Phase 5.
- Where the memory CLI/UX lives (TUI `/memory` commands) — Phase 9.

## 9. Stopping criteria

Phase 1 is complete when:

- [x] All 7 framework/layer reports written (§1–§8).
- [x] Comparison matrix + constraints mapping written (§9–§33).
- [x] Architecture doc with Mermaid diagram, A/B/C recommendation, and
      Tier 1/2/3 designs written (§34–§37).
- [x] Final report with §42 structure written.
- [x] `phases.md` state refreshed ("🔬 Memory Research Phase 1 — IN PROGRESS").
- [x] Warp checkout verified clean (`git status --porcelain=v1` empty).
- [ ] **Research STOP** — no implementation, no next phase.