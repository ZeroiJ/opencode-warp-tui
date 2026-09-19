# §9–§33 — Comparative analysis

> Evidence: framework sections §2–§8 (this report's sibling docs) plus the
> live OpenCode 2.0.8 probes. Labeling: **[VERIFIED]** primary
> source/live · **[OBSERVED]** seen, not confirmed · **[INFERRED]**
> analysis.

---

## §9 Capability matrix

| Dimension | Hermes (builtin) | Hindsight | Mem0 | Letta | OpenViking | Holographic (provider) |
|---|---|---|---|---|---|---|
| License | MIT (agent)/plugin MIT | MIT | Apache-2.0 | Apache-2.0 | AGPL-3.0 | MIT |
| Language | Python | Go server + SDKs | Python | Python + TS harness | Python + Rust | Python |
| Storage | Files (MD) | Postgres + pgvector (pg0) | Qdrant/FAISS/etc + SQLite history | SQLite / Postgres + pgvector; MemFS git | AGFS/RAGFS + vector index | **SQLite + FTS5** |
| Embeddings | none (curated) | required (bge-small) | required (default OpenAI) | required (archival) | required (Doubao/Ollama) | **optional** (HRR = hash vectors) |
| Extraction | agent-self-managed | LLM extract (retain) | **LLM ADD-only** | agent tools + sleep-time | LLM extract + dedup (commit) | agent tool (auto_extract off) |
| Retrieval | in-context only | TEMPR (4-strategy + RRF + CE) | hybrid BM25 + semantic + entity | archival/recall search | intent → typed queries → hierarchical | **FTS5 + Jaccard + HRR**, trust-weighted |
| Contradiction | replace w/ failure cap | **refine, preserve history** | LLM dedup (explicit update) | replace/rewrite blocks | merge/delete via dedup LLM | **heuristic, surfaced** |
| Consolidation | agent budgets | observations background | V3 adds only | sleep-time 2nd agent | async commit consolidation | trust feedback |
| Context model | bounded blocks ≤2200/1375 chars | token budgets (max 4096) | top_k + threshold | 3-tier + 80% budget + compaction | **L0/L1/L2 progressive** | fact rows, min_trust |
| OpenCode integration | n/a (Hermes-private) | official plugin + MCP + HTTP | official plugin (TS) | server (API/WS/OpenAI-compat) | official plugin + MCP + HTTP | n/a (Hermes-private) |
| Experiment-hook dependence | n/a | plugin uses session.idle | hooks + tools | n/a (separate server) | plugin auto-recall | n/a |

**[VERIFIED]** rows compose facts from §2–§8. **[INFERRED]** the last row.

---

## §10 Architecture model

| Model | Systems |
|---|---|
| Agent-loop-coupled (tools the agent calls) | Hermes builtin, Letta (memory tools), Holographic provider |
| External server (HTTP/MCP, language-neutral) | Hindsight, OpenViking, Letta server, Honcho |
| Library/factory (embed in host) | Mem0 SDK, Holographic store core |
| Sidecar reacts to session events | Hindsight plugin, OpenViking plugin, most OpenCode plugins |

**[VERIFIED]** — from §2–§8. **[INFERRED]** For `opencode-warp-tui` the
**external-server/sidecar** and **library** models both fit the adapter
boundary; the agent-loop-coupled model needs the agent to actually call
tools, which our TUI can't force.

---

## §11 Storage

- **File-based curated** (Hermes MD, many OpenCode plugins): human-readable,
  git-able, no deps; weak for scale/search. **[VERIFIED]**
- **SQLite + FTS5** (Holographic, most OpenCode plugins, Letta default):
  single file, zero services, built-in full-text; embedding blobs optional
  (engram pattern). **[VERIFIED]**
- **Embedded vector** (OpenViking local, opencode-mem Turso/libSQL):
  native vector ops in-process; vendor-specific SQL extensions. **[VERIFIED]**
- **External Postgres + pgvector** (Hindsight, Letta prod): excellent at
  scale; heavy for local-first. **[VERIFIED]**
- **[INFERRED]** Recommended: **SQLite + FTS5 for Tier 2**, vector as an
  opt-in column/extension later, files for Tier 1.

## §12 Retrieval

- FTS5 keyword (Holographic) — cheap, deterministic.
- BM25 hybrid + entity boost (Mem0) — strong keyword+semantic fusion.
- TEMPR 4-strategy + RRF + cross-encoder (Hindsight) — best evidence
  quality, most moving parts.
- HRR algebraic (Holographic) — zero-model vector-lite; unproven quality.
- Hierarchical progressive (OpenViking) — token-efficient browsing.
- **[INFERRED]** Start: FTS5 + lightweight ranking (trust/recency),
  add optional embeddings later; RRF when fusing ≥2 signals.

## §13 Extraction & lifecycle

| Trigger | Systems |
|---|---|
| Agent self-edits (tools) | Hermes, Letta, Holographic, most simple plugins |
| Explicit API call | Hindsight retain, OpenViking commit |
| Session boundary (end/compact) | Hermes on_session_end/on_pre_compress; OpenCode plugins on compacting |
| Background consolidation | Hindsight observations, Letta sleep-time, OpenViking async commit |
| Hook per-prompt | mem0 (searches every prompt) |

**[VERIFIED]** — across §2–§8. **[INFERRED]** For our adapter: capture
events (message/step/compact) on the TUI side; extract at **session end
and compaction**, never per-prompt (token discipline).

## §14 Memory types & scopes

- Types: world/experience/observation/model (Hindsight); facts +
  categories user_pref/project/tool/general (Holographic); 9 types incl.
  identity/soul/trajectories (OpenViking); persona/human/domain blocks
  (Letta); decision/learning/preference/… (community plugins).
- Scopes: user × agent × run (Mem0); bank/tag visibility (Hindsight);
  user/session/repo tiers (lkonga); user-level + project + session
  (OpenViking resources vs memories).
- **[INFERRED]** Project fit: scopes **user (global) / project (repo) /
  session (episodic)**, types **facts, preferences, decisions, entities,
  experiences** to start.

## §15 Contradiction & consolidation

- Refine-with-history (best; Hindsight, Holographic heuristic);
  overwrite-by-LLM-decision (Mem0 explicit update; Letta blocks);
  ADD-only-accumulate (Mem0 V3 — avoids the decision loop but needs a
  dedup/consolidation pass); agent-driven replace with failure budget
  (Hermes).
- **[INFERRED]** Design: **ADD + evidence history + surfaced conflicts**;
  explicit user/agent overwrite only via visible tool; never silent
  overwrite.

## §16 Token/context management

- Hermes: hard char budgets + frozen prompt snapshot (prefix-cache).
- Letta: 80% context budget, reactive compaction, lighter summarizer.
- Hindsight: `max_tokens`, budget levels.
- OpenViking: L0/L1/L2 progressive loading, `recallTokenBudget` (2000),
  `recallMaxContentChars` (500).
- **[INFERRED]** Adopt: **char-budgeted blocks in-context + token-budgeted
  recall + progressive disclosure**; keep prefix-cache stability in mind
  (Hermes snapshot pattern).

## §17 Privacy & secrets

- Local-only defaults: Holographic (zero network), Hermes (local files),
  Letta SQLite, Hindsight local stack, OpenViking local backend.
- Cloud/telemetry: Mem0 PostHog telemetry + cloud hosted option; Supermemory
  cloud; Honcho cloud. **[VERIFIED]**
- **[INFERRED]** Our engine must be **local-only by default**, secret
  redaction before storage, and no telemetry.

## §18 User control

- Commands: Letta `/palace` `/doctor`; working-memory `/memory`;
  mem0 9 skills incl. `/mem0-forget`; Holographic `fact_feedback`;
  Hermes memory tool add/remove.
- **[INFERRED]** Provide list/inspect/delete/export + opt-out of
  auto-capture; UX lands in Phase 9.

## §19 OpenCode compatibility

- Verified integration surface (live 2.0.8): HTTP session/message APIs,
  instruction entries (experimental), SSE events, plugin API (stable),
  `experimental.session.compacting`. **[VERIFIED]**
- `experimental.chat.system.transform` absent from current docs — at risk.
  **[OBSERVED]**
- **[INFERRED]** Keep integration behind the adapter: capture = SSE
  events; injection = supported boundaries (system prompt attachments /
  instructions entries / synthetic), never undocumented hooks.

## §20 Local-first

| System | Fully local? | Notes |
|---|---|---|
| Hermes builtin | ✅ | files only |
| Holographic | ✅ | SQLite, numpy optional |
| Letta | ✅ | SQLite default |
| Mem0 | ⚠️ | local config possible; defaults cloud |
| Hindsight | ⚠️ | local stack possible; Postgres/pg0 heavy |
| OpenViking | ⚠️ | local vector backend + Ollama possible; Doubao-centric |

**[VERIFIED]** across §2–§8.

## §21 Licenses & code reuse

- MIT: Hermes (agent), Hindsight, Holographic plugin, most community
  plugins — **safe to reference/copy with attribution**.
- Apache-2.0: Mem0, Letta — safe to reference/copy with attribution.
- AGPL-3.0: OpenViking core (Apache-2.0 CLI/examples), this project's own
  code is AGPL-3.0-only overall — **copying OpenViking core obligates
  AGPL distribution**; prefer reference-only. **[INFERRED]**

## §22 Complexity & maturity

| System | Maturity | Complexity |
|---|---|---|
| Hermes builtin | production (their agent) | low |
| Hindsight | v0.x, active, plugin published | medium-high (Postgres) |
| Mem0 | v2.1, very active, production cloud | medium (factory stack) |
| Letta | very active, production platform | high (full platform) |
| OpenViking | pre-stable 0.3.x, very active | high (FS + sidecar) |
| Holographic plugin | merged, unmaintained standalone | low |

**[VERIFIED]** star counts/versions from §2–§8. **[INFERRED]** maturity is
not the deciding factor; **pattern fit + integration cost** is.

---

## §23–§33 Constraints mapping for THIS project

### §23 Local-first (no mandatory cloud)

- **Verdict**: exclude any design where cloud is the default or required.
  Mem0/Hindsight/OpenViking/Honcho all *can* run local but bias toward
  hosted (defaults, benchmarks, docs). Hermes builtin / Holographic / Letta
  are local-native. **[INFERRED]**
- **Leads to** §34: local store (SQLite/FTS5), offline extraction fallback.

### §24 No OpenCode fork; minimal invasive modification

- OpenCode must keep working across upgrades. Experimental surfaces
  (instruction entries, compacting hook) are usable but must be
  **feature-flagged and non-fatal when absent**. **[INFERRED]**

### §25 Adapter isolation (memory must not touch the TUI)

- Memory engine communicates through the existing `Backend` boundary:
  ingest from `StreamEvent`/session snapshots; expose memory context to the
  adapter's request path — or lives behind a separate trait usable by the
  request builder. **[INFERRED]**

### §26 No Cargo dependencies this phase / impl deferred

- All dependencies presumed for Phase 5+; research must state them
  explicitly so Phase 5 can pick (e.g., `rusqlite`, `tiktoken`-class
  token counting). **[INFERRED]**

### §27 SQLite-compatible baseline

- Holographic's fact schema, OpenCode plugins' FTS5+blob pattern, Letta's
  SQLite default — all converge on SQLite + FTS5 + optional vector
  column. **[VERIFIED]** / **[INFERRED]**

### §28 Token discipline

- Adopt Hermes budgets + Hindsight budget levels + OpenViking
  progressive disclosure. In-context Tier 1 stays small and frozen per
  session (prefix-cache). **[INFERRED]**

### §29 AGPL-3.0-only crate

- Only MIT/Apache code should be copied (with attribution);
  AGPL frameworks (OpenViking core) remain reference-only. **[INFERRED]**

### §30 Privacy & secrets

- Local-only store; redact secrets at capture (patterns + opt-in LLM
  review); no telemetry; export/wipe-friendly formats. **[INFERRED]**

### §31 User control

- Visibility of what is stored, where it came from (provenance), and how
  it was scored; explicit delete/forget; opt-out of auto-capture.
  **[INFERRED]**

### §32 Version resilience

- Detect OpenCode version at connect (already done in Phase 4:
  `OpenCodeBackend.connect` verification) and gate experimental features
  on it. **[INFERRED]**

### §33 Synthesis

**Convergence** (all framework evidence points the same way):

1. **Tier 1** — bounded in-context curated memory (Hermes-style blocks,
   Letta-style tools) — always-present, small, agent-editable, frozen
   per session for prefix-cache.
2. **Tier 2** — local **SQLite + FTS5** fact store (Holographic schema
   + Hindsight metadata + Mem0 hash-dedup + scopes) with hybrid retrieval
   (FTS5 + recency/trust ranking, later optional embeddings + RRF),
   contradiction **surfaced not overwritten**, trust feedback.
3. **Tier 3** (future) — semantic/graph: optional sidecar embeddings
   (Ollama/HRR hashes), entity graph (Mem0 style), progressive L0/L1/L2
   (OpenViking style), background consolidation (Letta sleep-time scaled
   down).

The architecture document (§34–§37) turns this synthesis into diagrams,
component specs, an A/B/C recommendation, and the tier designs.