# §6 — OpenViking (volcengine)

> Evidence: official repo `github.com/volcengine/OpenViking`, official docs
> `docs.openviking.ai` (concepts pages), papers (VikingMem arXiv:2605.29640,
> TrieHI arXiv:2606.16903, VikingRAG arXiv:2609.11390). Subagent deep-dive
> notes in `/tmp/opencode/subagent-notes/openviking.md`.

---

## 6.1 Identity

| Attribute | Value | Label |
|---|---|---|
| Repo | `volcengine/OpenViking` (ByteDance) | **[VERIFIED]** |
| License | AGPL-3.0 (core); Apache-2.0 (CLI + examples) | **[VERIFIED]** |
| Language | Python (core), Rust (`crates/`, RAGFS), Go/TS SDKs | **[VERIFIED]** |
| Stars | ~38k ⭐ / ~2.9k forks (Sep 2026) | **[VERIFIED]** |
| Activity | Very high — multiple daily commits | **[VERIFIED]** |
| Version | Pre-stable (0.3.x); APIs can break | **[OBSERVED]** |
| Server | Standalone sidecar HTTP on port **1933** | **[VERIFIED]** |

## 6.2 Core model: context database as a virtual filesystem

OpenViking unifies memory, resources, and skills in a **`viking://`
virtual filesystem**:

- **Resources** — user-added knowledge (docs, code, PDFs); static,
  user-driven.
- **Memory** — agent-learned knowledge from interactions; dynamic,
  agent-driven. Nine types: `profile`, `preferences`, `entities`, `events`,
  `identity`, `soul`, `cases`, `trajectories`, `experiences`.
- **Skills** — declarable agent capabilities under
  `viking://~/skills/{name}/`.

**[VERIFIED]** — docs.openviking.ai `/en/concepts/02-context-types`.

## 6.3 L0/L1/L2 progressive loading

**[VERIFIED]** — `/en/concepts/03-context-layers`.

| Layer | Name | Body limit | Purpose |
|---|---|---|---|
| L0 | Abstract | 256 chars | vector retrieval, quick filtering |
| L1 | Overview | 4000 chars | rerank, navigation, planning |
| L2 | Detail | unlimited | full content, on demand |

- L0/L1 are **directory-level semantic sidecars** (`.abstract.md`,
  `.overview.md`), not per-file.
- Generated bottom-up: file summaries → leaf L1 → leaf L0 → parent L1 →
  parent L0; child L0 bodies aggregate into parent L1.
- Agents check L0 for relevance → L1 for scope → load L2 only when needed.
  This "progressive loading" directly saves tokens.
- Freshness tracking: `total_entries`, `sampled_entries`,
  `unsampled_entries`, `pending_child_changes`; stable sampling when
  direct children > 32.

## 6.4 Session commit / memory extraction

**[VERIFIED]** — `/en/concepts/08-session`.

Lifecycle: Create → Interact → **Commit** (two phases):

1. **Synchronous** — increment compression index, write messages to
   archive, clear current messages, return `task_id` immediately.
2. **Asynchronous background** — generate structured summary → extract
   long-term memories → write `memory_diff.json` → update `active_count`
   → write `.done` marker.

Extraction flow:
`Messages → LLM Extract → Candidate Memories → Vector Pre-filter (find
similar) → LLM Dedup Decision → Write to AGFS → Vectorize`.

- Dedup decisions: `skip` / `create` / `none` (per candidate); `merge` /
  `delete` (per existing item).
- Every commit writes `memory_diff.json` (adds/updates/deletes/skipped)
  as an **audit trail** with rollback capability.
- `experiences` type activates the full Agent Evolution pipeline
  (cases + trajectories).
- Compressor keeps recent N rounds; archives older messages.

**[INFERRED]** This two-phase sync/async commit is a strong lifecycle
pattern for OpenCode sessions: the TUI/adapter completes the synchronous
archive immediately (cheap), and extraction runs in background (or at
session end) without blocking the UI.

## 6.5 Retrieval: intent analysis → typed queries → hierarchical search

**[VERIFIED]** — `/en/concepts/07-retrieval`.

- Two modes: `find()` (simple, single query, no session context, low
  latency) vs `search()` (complex, generates **0–5 TypedQueries** via LLM
  intent analysis; higher latency).
- Intent analysis input: query + session compression summary + last 5
  messages; outputs queries with `query`, `context_type`
  (MEMORY/RESOURCE/SKILL), `intent`, `priority`.
- Hierarchical retrieval: priority-queue recursive directory search over
  `viking://~/memories`, `viking://resources`, `viking://~/skills`;
  global vector search (top-10) → merge + rerank → recursive descent with
  score propagation; convergence detection (stop if top-k unchanged for 3
  rounds); `score_propagation_alpha = 1.0`.
- Rerank via Volcengine `doubao-seed-rerank` (falls back to vector scores).

## 6.6 Storage & vector index

**[VERIFIED]** — `/en/concepts/05-storage`.

- Dual-layer: **AGFS/RAGFS** (Rust) stores full L0/L1/L2 content; **Vector
  index** stores only URIs, vectors, metadata.
- Vector backends: `local`, `http`, `volcengine` (managed VikingDB).
- Index strategy: `flat_hybrid`, cosine, int8 quantization; sparse vectors
  supported.
- Embedding models: Doubao default; OpenAI, Codex OAuth, Kimi, GLM,
  **local Ollama** supported → full local operation possible.

## 6.7 Token efficiency benchmarks

**[VERIFIED]** — README claims (self-reported):
LoCoMo accuracy 80–83% vs 24–57% native memory; input tokens −34.3 to
−91.0%; query latency −58.45 to −66.10%; tau2-bench +6.87pp retail,
+11.87pp airline. Mechanisms: L0/L1/L2 progressive loading, directory-level
sidecars, hierarchical retrieval, session compression, `recallTokenBudget`
(default 2000) and `recallMaxContentChars` (default 500).

## 6.8 OpenCode integration

**[VERIFIED]** — `/en/agent-integrations/10-opencode`.

- npm plugin `@openviking/opencode-plugin` + MCP server (stdio proxy);
  one-line installer script exists.
- 15 MCP tools: `openviking_find/search/read/list/tree/grep/glob`,
  `openviking_remember/write/edit/add_resource`,
  `openviking_list_watches/cancel_watch/forget/health`.
- Config: `timeoutMs: 30000`, `repoContext: true`,
  `repoContextCacheTtlMs: 60000`; shared `recallLimit: 6`,
  `scoreThreshold: 0.35`, `recallPreferAbstract: true`,
  `commitTokenThreshold: 20000`, `commitKeepRecentCount: 10`,
  `profileTokenBudget: 10000`, `resumeContextBudget: 32000`.
- Sidecar HTTP server on port 1933 → **any agent (or adapter) can connect
  via HTTP or MCP**.

## 6.9 Assessment for OpenCode — adopt / avoid

### Adopt ([VERIFIED] concepts)

1. **L0/L1/L2 progressive loading** with strict body limits — proven token
   reduction; generalizes to any memory system (abstract ≤256 chars,
   overview ≤4000).
2. **Session commit → async extraction → dedup → diff audit** — two-phase
   lifecycle with `memory_diff.json` rollback story.
3. **Intent analysis → typed queries → hierarchical retrieval** — more
   targeted than flat vector search (0 queries for chit-chat saves work).
4. **Memory-type taxonomy** — a small set of typed memories improves
   retrieval precision (careful not to overreach: 9 types may be too many
   for v1).
5. **MCP/sidecar architecture** — language-neutral interface; any client
   can connect.
6. **Recall budget knobs** (`recallTokenBudget`, `recallMaxContentChars`,
   `recallPreferAbstract`) — explicit bounded injection.

### Avoid / modify

7. **Doubao/Volcengine coupling** — primary embeddings/rerank are
   ByteDance-managed; local-via-Ollama works but the project's benchmarks
   and managed SaaS are Volcengine-centric. **[OBSERVED]**
8. **AGPL-3.0 core** — license-compatibility check needed for any code
   reuse; treat as architecture reference, not vendored code. **[INFERRED]**
9. **Pre-stable churn** (0.3.x, rapidly changing APIs; community notes
   features dropped for compatibility) — do not depend on its HTTP
   contract. **[OBSERVED]**
10. **Nine memory types + full Agent Evolution pipeline** — start smaller
    (facts/preferences/decisions/entities), grow later. **[INFERRED]**