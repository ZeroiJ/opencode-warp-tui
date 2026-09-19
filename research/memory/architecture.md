# §34–§37 — Memory system architecture (candidate)

> Status: 🔬 RESEARCH OUTPUT ONLY — a candidate architecture for review in
> Memory Research Phase 2. **Nothing here is implemented.**
> Labels: **[VERIFIED]** primary source/live probe ·
> **[OBSERVED]** seen but not confirmed · **[INFERRED]** analysis.
> The hypothesis diagram in `phases.md` (HYPOTHESIS — NOT FINAL
> ARCHITECTURE) is refined and expanded here.

---

## ⚠️ Phase 2B addendum (2026-09-18) — this document is a *candidate*

Phase 2A (`phase2a-memory-model.md`, `minimal-architecture.md`,
`decision-log.md` D1–D14) reviewed and minimized the candidate below;
Phase 2B (`phase2b-decision-report.md` + five companion docs,
`decision-log.md` D15–D24) **resolved it into the final architecture**.

**The six-component engine, eight types, three scopes, five states,
scores, and SQLite/FTS5 described in §34–§37 below are NOT the V1 plan.**
The final V1 is: Memory API + file-backed MemoryStore + Context Builder,
two kinds (fact|preference), two scopes (user|project), ACTIVE/
SUPERSEDED (DELETED reserved), zero scores, JSONL, one frozen
instruction-entry injection per session. `phase2b-decision-report.md` §2
is the authoritative final architecture diagram; this document remains on
disk unmodified as Phase 1's candidate for the historical record.

---

## §34 Candidate architecture

### 34.1 Design goals (recap)

- Local-first, private by default, zero mandatory cloud. **[INFERRED]**
- OpenCode-compatible without forking or invasive modification; survives
  upgrades. **[INFERRED]**
- Memory engine attaches at the **adapter boundary** (behind the
  `Backend` trait), not inside TUI rendering. **[INFERRED]** (AGENTS.md
  rule)
- Bounded token use: in-context Tier 1 small + frozen per session;
  Tier 2/3 retrieval token-budgeted. **[INFERRED]**
- Facts and inferences are stored and labeled as such (this property must
  survive into the engine itself). **[INFERRED]**

### 34.2 Overall architecture (Mermaid)

```mermaid
flowchart LR
    subgraph OC["OpenCode 2.x (live server, HTTP + SSE)"]
        API["REST /api/session · context · instructions · generate"]
        EVT["SSE events: session.message.* · session.next.* · compact"]
    end

    subgraph TUI["opencode-warp-tui (Warp-style TUI)"]
        RT["TuiRuntime / App"]
        BT["Backend trait"]
        OB["OpenCodeBackend (adapter)"]
    end

    subgraph ME["Memory Engine (backend-layer, Phase 5+)"]
        CAP["Capturer<br/>reads session events / snapshots"]
        EXT["Extractor<br/>segmenter · rule extractor · LLM extractor (opt-in)"]
        STORE["Store<br/>SQLite + FTS5<br/>facts · prefs · decisions · entities · provenance"]
        RET["Retriever<br/>FTS5 · recency · trust · (later: vector + RRF)"]
        CON["Consolidator<br/>dedup · contradiction surface · trust feedback"]
        CTX["Context Builder<br/>token budget · scope filter · fact-vs-inference labels"]
    end

    subgraph TIER1["Tier 1 — in-context curated memory (files)"]
        BLOCKS["MEMORY.md / USER.md (char-budgeted §-blocks)"]
    end

    OC -->|HTTP/SSE| OB
    OB -->|StreamEvent / snapshots| RT
    OB -->|session events| CAP

    CAP --> EXT
    EXT --> STORE
    STORE --> CON
    CON --> STORE
    RET --> CTX
    STORE --> RET

    CTX -->|budgeted, labeled memory context| OB
    OB -->|system prompt / instructions entries / attachments| API

    TIER1 -->|always-in-context, frozen per session| CTX
    RET -->|Tier 2 / Tier 3 recall| CTX

    ME -.="reads only (no schema changes)"-.-> OC
```

**[VERIFIED]** — OpenCode side (HTTP/SSE, session events, instruction-entry
endpoint) from live 2.0.8 probes. Everything inside `ME`/`TIER1` is a
**candidate design** **[INFERRED]** for Phase 2/5 review.

### 34.3 Data flow in words

1. **Capture** — the adapter already receives OpenCode events
   (`StreamEvent`, session snapshots). The Capturer selects memory-relevant
   turns (message ends, tool results, decisions, user preferences) and
   writes them to a session log. **[INFERRED]**
2. **Extract** — at session end / compaction, the Extractor turns the log
   into candidate memories: segmentation + rule extraction always; LLM
   extraction behind an opt-in flag (requires a configured model; falls
   back to rules when absent). **[INFERRED]**
3. **Store** — candidates are hashed (md5-style dedup, Mem0 pattern
   **[VERIFIED]** concept), scoped (user/project/session), tagged
   fact-vs-inference, timestamped (`occurred` vs `learned`, Hindsight
   pattern **[VERIFIED]** concept) and written to SQLite+FTS5. **[INFERRED]**
4. **Consolidate** — duplicates merge (evidence count grows), conflicting
   facts are flagged/surfaced rather than overwritten (Holographic
   heuristic + Hindsight refinement **[VERIFIED]** concepts). Trust scores
   move with explicit feedback (+small/−large, clamped). **[INFERRED]**
5. **Retrieve** — on session start (and per compaction), the Retriever
   runs scoped queries: FTS5 + recency + trust ranking; optional
   embeddings + RRF later (Tier 3). **[INFERRED]**
6. **Build context** — the Context Builder renders a **budgeted**,
   **labeled** block (facts vs inferences, with provenance) and hands it to
   the adapter for injection through supported OpenCode boundaries only
   (see §35.6). **[INFERRED]**

### 34.4 Claims and their status

| # | Claim | Status |
|---|---|---|
| C1 | OpenCode exposes a live HTTP/SSE surface usable for capture and injection | **[VERIFIED]** (2.0.8 probes) |
| C2 | OpenCode has no built-in memory | **[VERIFIED]** (issues #24030/#20322) |
| C3 | A memory engine at the adapter layer can observe every session it serves | **[INFERRED]** — requires event coverage check in Phase 5 |
| C4 | Injection via instructions entries is possible | **[VERIFIED]** (live round-trip) — but experimental |
| C5 | Injection via plugin system-prompt hooks is durable | **[OBSERVED]** — `chat.system.transform` absent from current docs |
| C6 | SQLite+FTS5 covers Tier 2 retrieval requirements | **[INFERRED]** — sufficient for keyword+recency+trust |
| C7 | No Cargo dependency is needed to prototype this in Phase 5 | **[INFERRED]** — Phase 5 will evaluate `rusqlite` etc. |

---

## §35 Component specifications (candidate)

Each component: responsibility, key inputs/outputs, OpenCode boundary
touched, and open questions. **[INFERRED]** designs unless marked.

### 35.1 Capturer

- **Responsibility**: subscribe to adapter-side session events; maintain a
  lightweight session log (`session_id`, turn index, role, text, tool
  names, timestamps, project path).
- **Inputs**: `StreamEvent`s and session snapshots via the existing
  `Backend` vocabulary (Phase-4 mapper).
- **Boundary**: none (TUI-internal, adapter-adjacent); no new OpenCode
  surface. **[INFERRED]**
- **Open questions**: how to redact secrets at capture (pattern-based v1);
  how much raw text to keep before extraction (bounded ring buffer).

### 35.2 Extractor

- **Responsibility**: segment session log → candidate memories.
- **Modes**:
  - Rules only (default): sentence/decision patterns, quoted preferences,
    tool-result summaries, explicit phrases ("remember/forget that…").
  - LLM-assisted (opt-in): extraction prompt modeled on Mem0's ADD-only
    discipline **[VERIFIED]** concept; JSON-schema output (Hindsight
    pattern **[VERIFIED]** concept).
- **Outputs**: memory records `{text, type, scope, fact/inference,
  occurred_at, learned_at, source_ref, entities[]}`.
- **Dedup**: `md5(text)` + normalized entity overlap.
- **Failure policy**: non-fatal; never blocks the user turn (Hermes
  consolidation-failure-cap spirit **[VERIFIED]** concept).

### 35.3 Store (SQLite + FTS5)

- Tables (candidate, informed by Holographic schema **[VERIFIED]** +
  Mem0 payload fields **[VERIFIED]**):
  - `memories(id, text, type, scope_kind, scope_id, fact_inference,
    trust, proof_count, retrieval_count, created_at, occurred_at,
    updated_at, status[active|superseded|flagged_conflict])`
  - `entities(id, name, kind, memory_ids[])` (JSON or link table)
  - `provenance(id, memory_id, session_id, turn_id, ref_kind, payload)`
  - `events(id, memory_id, event[add|feedback|conflict|supersede],
    detail, created_at)` — audit trail (Mem0 history **[VERIFIED]**)
  - FTS5 virtual table over `text` (+ `type`, scope columns).
- **Storage rules**: local file (project-local `.owt/` or user-level dir —
   Phase 2 decision); no OpenCode schema changes. **[INFERRED]**

### 35.4 Retriever

- Scope-filtered queries (user/project/session); ranking = FTS5 BM25-ish
  score × recency decay × trust (multiplicative, Hindsight-style boost
  discipline **[VERIFIED]** concept, but conservative ±).
- **Hybrid v1**: keyword (FTS5) + recency + trust. **Hybrid v2 (Tier 3)**:
  add embeddings + RRF fusion.
- Token-budgeted: `max_chars` per scope; progressive disclosure
  (id → short list → full detail on demand, OpenViking L0/L1/L2 spirit
  **[VERIFIED]** concept).
- Conflict surfacing: when a query touches `flagged_conflict` memories,
  return both sides with provenance (Hindsight refinement **[VERIFIED]**
  concept).

### 35.5 Context Builder

- Renders the final memory block for injection:
  - **Fenced** (`<memory-context>…NOT new user input…</memory-context>` —
    Hermes **[VERIFIED]** pattern).
  - **Labeled** — each item tagged `[fact]`/`[inference]` with scope and
    date (provenance on request).
  - **Budgeted** — hard char budget per tier and per scope; the in-context
    Tier 1 block is **frozen per session** (prefix-cache, Hermes snapshot
    **[VERIFIED]** pattern).
- **Injection boundaries** (order of preference, all behind a version
  gate): (a) instruction entries API — **[VERIFIED]** live but
  experimental; (b) prompt attachments/files (docs-verified surface);
  (c) plugin `system.transform` — **[OBSERVED]** ecosystem-standard but
  absent from current official docs ⇒ feature-flagged; (d) synthetic
  message at session start — **[VERIFIED]** endpoint exists, UI-visible
  trade-off. **[INFERRED]** final choice is Phase 2 review.

### 35.6 TUI visibility

- Phase 9 UX only: `/memory list|show|search|forget|export`;
  statusline indicator when memory was injected (Hermes recall indicator
  **[VERIFIED]** pattern). Not built now. **[INFERRED]**

---

## §36 Recommendation — Options A/B/C

> The phases.md hypothesis (HYPOTHESIS — NOT FINAL ARCHITECTURE) is
> resolved into three coherent options. Recommendation is **B with A's
> Tier 1 and C's isolation principles**; A/B/C reviewed in Phase 2.

### Option A — Curated in-context memory only (zero infrastructure)

- What: Tier 1 only. MEMORY.md/USER.md-style §-blocks (char-budgeted),
  edited by the user (TUI commands) and/or agent tool calls; injected via
  supported boundaries. No SQLite, no extraction pipeline.
- Strengths: minimal, predictable, human-readable, git-able, no deps.
- Weaknesses: no search beyond what fits in-context; no project-scale
  recall; extraction/consolidation absent.
- Fit: bootstrap for Phase 5; not sufficient alone. **[INFERRED]**

### Option B — Adapter-side memory engine (SQLite + FTS5; recommended)

- What: Tier 1 + Tier 2 as designed in §35, living at the backend layer
  (not the TUI). Capture from adapter events; extraction rules-first with
  opt-in LLM; SQLite+FTS5 store; scope-aware retrieval; surfaced
  contradictions; budgeted, labeled context injection through verified
  OpenCode boundaries.
- Strengths: local-first; survives OpenCode upgrades (no experimental
  hooks required); aligns with AGENTS.md isolation; SQLite baseline
  confirmed by ecosystem (§8.3); token discipline built in.
- Weaknesses: largest in-crate build; events coverage must be verified
  (C3); extraction quality without an LLM is limited.
- **Recommendation: B primary.** **[INFERRED]**

### Option C — Sidecar memory service (HTTP/MCP)

- What: a separate local process exposing retain/recall/reflect-style HTTP
  or MCP (Hindsight/OpenViking model **[VERIFIED]** concepts); the adapter
  (or a plugin) calls it. Tier 3 capabilities (embeddings, graph) live in
  the sidecar.
- Strengths: language-neutral; ability to use embeddings without Cargo
  deps; decouples lifetime from the TUI process; ecosystem-proven.
- Weaknesses: second process to run/version; MCP or HTTP client code in
  the adapter; feature flag complexity; needs sidecar install story.
- Fit: **future extension of B** for embeddings/graph (Tier 3), not v1.
  **[INFERRED]**

### Comparative sketch

| Criterion | A | B | C |
|---|---|---|---|
| Local-first | ✅ | ✅ | ✅ |
| No experimental hooks required | ✅ | ✅ | ✅ (HTTP) |
| Survives OpenCode upgrades | ✅ | ✅ | ✅ |
| Adapter-isolated (AGENTS.md) | ✅ | ✅ | ⚠️ (plugin if used) |
| Scale (project-level recall) | ❌ | ✅ | ✅ |
| Token discipline | ✅ built-in | ✅ designed | ⚠️ config-dependent |
| Implementation cost | low | medium | medium-high |
| Embeddings/graph path | ❌ | via C later | ✅ native |

**[INFERRED]** evaluation weights: local-first + isolation + upgrade
resilience dominate ⇒ B.

---

## §37 Tier 1/2/3 designs

> Three escalating designs. Tier 1 = v1 (Phase 5); Tier 2 = v2; Tier 3 =
> Phase 8 aspiration. Each lists scope, storage, extraction, retrieval,
> injection, and explicit non-goals.

### Tier 1 — Curated in-context memory (v1)

- **Scope**: user/profile + current project conventions; ≤ 2 blocks;
  char budgets (e.g., 2200 / 1375, Hermes **[VERIFIED]** defaults).
- **Storage**: files (`MEMORY.md`, `USER.md`) in the project/user memory
  dir; §-delimited entries; atomic writes + drift guard (Hermes
  **[VERIFIED]** patterns).
- **Editing**: TUI slash commands (`/memory add|list|remove`) + an agent
  tool if the backend exposes tools to OpenCode. **[INFERRED]**
- **Injection**: rendered block appended to the system prompt through the
  supported boundary that Phase 2 selects (instruction entries /
  attachments / transform). Frozen per session for prefix-cache.
- **Non-goals**: search, agents deciding what to store automatically,
  cross-project recall.

### Tier 2 — Local fact engine (v2; recommended build)

- **Scope**: user × project × session; types facts/preferences/decisions/
  entities; fact-vs-inference label on every row.
- **Storage**: SQLite + FTS5 (candidate schema §35.3); audit `events`
  table; provenance (`session_id`, `turn_id`).
- **Extraction**: rules-first + opt-in LLM (ADD-only discipline, schema
  JSON, hash dedup). **[INFERRED]**
- **Retrieval**: scoped FTS5 + recency × trust ranking; budgeted,
  fenced, labeled context; conflict surfacing.
- **Consolidation**: at session end and compaction; duplicate merge,
  trust feedback (`+0.05/−0.10` clamp, Holographic **[VERIFIED]**
  defaults); contradiction → `flagged_conflict`, never silent
  overwrite.
- **Injection**: Context Builder (§35.5) through version-gated
  boundaries; `experimental.*` surfaces only when present and behind a
  flag (C4/C5).
- **Non-goals**: embeddings, semantic-only queries, background daemons.

### Tier 3 — Semantic + graph intelligence (Phase 8 aspiration)

- **Adds**: optional embeddings (local Ollama or deterministic HRR
  hash-vectors — Holographic **[VERIFIED]** pattern) via a **sidecar
  (Option C)** so no Cargo embedding deps land in the TUI crate;
  entity graph with boost (Mem0 **[VERIFIED]** concept); RRF fusion;
  L0/L1/L2-style progressive disclosure (OpenViking **[VERIFIED]**
  concept); sleep-time-style background consolidation (Letta
  **[VERIFIED]** concept, scaled: periodic, bounded, off-critical-path).
- **Non-goals**: cloud services; cross-device sync beyond git remotes;
  automatic memory without user visibility.