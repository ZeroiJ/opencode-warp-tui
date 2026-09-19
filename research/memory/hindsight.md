# §3 — Hindsight (vectorize-io)

> Evidence: official repo `github.com/vectorize-io/hindsight`, official docs
> `hindsight.vectorize.io`, paper `arXiv:2512.12818`, subagent deep-dive
> notes (`/tmp/opencode/subagent-notes/hindsight.md`).

---

## 3.1 Identity

| Attribute | Value | Label |
|---|---|---|
| Repo | `vectorize-io/hindsight` | **[VERIFIED]** |
| License | MIT | **[VERIFIED]** |
| Language | Go (server); Python/Node/Go clients | **[VERIFIED]** |
| Stars | ~23.9k ⭐ (Sep 2026) | **[VERIFIED]** |
| Latest | v0.10 docs / Go module v0.8.6 | **[VERIFIED]** |
| Storage | PostgreSQL 15+ + pgvector; embedded **pg0** single-binary mode | **[VERIFIED]** |
| Embedding default | `BAAI/bge-small-en-v1.5` (384-dim) via SentenceTransformers | **[VERIFIED]** |
| Reranker | `cross-encoder/ms-marco-MiniLM-L-6-v2` | **[VERIFIED]** |
| LLM providers | 25+ incl. Ollama, LM Studio, llama.cpp, OpenAI, Anthropic, Gemini | **[VERIFIED]** |
| Paper | arXiv:2512.12818 | **[VERIFIED]** |

## 3.2 Core model: Retain / Recall / Reflect

**[VERIFIED]** — official docs.

### Retain (capture + extraction)

- One call: `client.retain(bank_id, content, ...)`, or auto-triggered from
  plugin hooks / MCP/API.
- LLM extracts structured facts (entities, relationships, temporal data)
  from free text; output is schema-enforced JSON.
- Entity resolution via trigram similarity (`pg_trgm`); knowledge graph
  links (entity/temporal/semantic/causal).
- Two fact types: `world` (external facts) and `experience` (agent's own
  first-person history).
- Extraction modes: `concise`, `verbose`, `custom`, `verbatim`, `chunks`
  (chunks = raw text embedding with no LLM call).
- Background observation consolidation runs asynchronously.

### Recall (retrieval)

- **TEMPR**: four parallel retrieval strategies —
  1. Semantic (pgvector HNSW)
  2. Keyword BM25 (5 backends: native/vchord/pg_textsearch/pgroonga/pg_search)
  3. Graph (entity/temporal/causal link traversal)
  4. Temporal (time-range filtering)
- Fused with **Reciprocal Rank Fusion (RRF, k=60)** → cross-encoder
  reranking → multiplicative boosts:
  `final_score = CE_normalized × recency_boost × temporal_boost × proof_count_boost`
- Token budget control: `max_tokens` (default 4096); budget levels
  low(100)/mid(300)/high(1000).

### Reflect (synthesis)

- Agentic reasoning loop (up to 10 iterations) over mental models →
  observations → raw facts, with tools (`search_mental_models`,
  `search_observations`, `recall`, `expand`, `done`).
- Disposition traits (skepticism, literalism, empathy on 1–5) shape the
  reasoning; directives are hard rules.
- Freshness-aware: stale observations are verified against raw facts.
- Can return text + `based_on` evidence + structured output (JSON Schema).

## 3.3 Memory representation and types

| Type | Meaning | Example |
|---|---|---|
| World facts | External facts | "Alice works at Google" |
| Experiences | Agent first-person | "I patched the auth bug" |
| Observations | Consolidated, evidence-backed beliefs | "Alice is a Python-focused developer" |
| Mental models | Standing answers to recurring questions | "User prefers dark mode" |

**[VERIFIED]** — fact schema fields: `content`, `fact_type`, `timestamp`
(`occurred_start`/`occurred_end`), `learned_at`, `bank_id`, `entities`,
`relationships`, `tags`, `attachments`, `proof_count`, `consolidated_at`,
`freshness`.

Two time dimensions: **when it happened** (`occurred_*`) vs **when it was
learned** (`learned_at`). Temporal search parses queries into date windows;
recency/proximity are ±10% multiplicative boosts.

## 3.4 Contradiction handling: refinement, not overwrite

**[VERIFIED]** — Observations docs.

- Near-duplicates (cosine ≥ 0.97, `HINDSIGHT_API_CONSOLIDATION_DEDUP_THRESHOLD`)
  trigger reconciliation.
- Contradictory evidence is folded into the observation with history
  preserved: *"User was previously a React enthusiast... but has now switched
  to Vue"*; *"Alice works at Meta (previously thought to work at Google)"*.
- Raw facts are always retained; corrections are traceable.

## 3.5 Confidence / trust

Hindsight deliberately does **not** use a per-memory numeric confidence
score. Instead: **proof_count** (logarithmic: `clamp(0.5 + ln(n)/10, 0, 1)`
of supporting facts), plus query-time relevance (cross-encoder score, RRF
rank). The system is query-aware rather than statically scored.

**[INFERRED]** This keeps stored data neutral and biases ranking by
support evidence at retrieval time. Transferable as "evidence-weighted
retrieval" rather than "confidence-stamped storage".

## 3.6 OpenCode integration

**[VERIFIED]** — official docs + npm.

- Official npm plugin: `@vectorize-io/opencode-hindsight` (v0.2.8 verified
  on npm).
- Model: HTTP client (HindsightClient) → local `http://localhost:8888`; MCP
  server built-in at `/mcp/{bank_id}/`.
- Plugin registers tools `hindsight_retain`, `hindsight_recall`,
  `hindsight_reflect`.
- **Auto-retain on `session.idle`**; auto-recall on session start.
- Bank IDs: `opencode` default; dynamic with `HINDSIGHT_DYNAMIC_BANK_ID`.
- Adapter feasibility without forking: very high — the system is purely
  HTTP/REST; any language can call `POST /v1/{bank}/retain|recall|reflect`.

## 3.7 Local-first assessment

**[VERIFIED]** — official docs/README.

- `pip install hindsight-all` bundles server + DB (embedded pg0) + local
  LLM option (llama.cpp auto-downloads ~3.5 GB GGUF; or Ollama/LM Studio).
- Fully local stack works: models download from HuggingFace on first run.
- Docker one-liner recommended for production.

**[INFERRED]** pg0 (a full embedded PostgreSQL) is heavy for this project's
local-first goals; SQLite is an order of magnitude lighter. The
**retain/recall/reflect API shape** is the transferable part, not the
Postgres stack.

## 3.8 Assessment for OpenCode — adopt / avoid

### Adopt ([VERIFIED] concepts)

1. **Three-operation API surface** (retain/recall/reflect): cleanest
   minimal contract for any memory engine.
2. **TEMPR-style fused retrieval** (semantic + keyword + temporal at
   minimum) with **RRF fusion** — rank-based, simple, robust.
3. **Observations consolidation** — background dedup of facts into
   evidence-backed beliefs is the key differentiator from plain RAG.
4. **Contradiction via refinement with preserved history** — memory, not a
   cache.
5. **Token-budgeted recall** (`max_tokens`, budget levels) — bounded
   injection is essential for OpenCode context windows.
6. **Mental models** (pre-computed standing answers) — strong fit for a
   coding assistant's project conventions.
7. **`chunks` extraction mode** (no-LLM path) — useful escape hatch when a
   local LLM is unavailable.
8. **Strict JSON-Schema extraction** (`json_schema strict: true`) — reliable
   fact extraction, important with local models.

### Avoid / modify

9. **PostgreSQL + pgvector as the base** — too heavy for local-first; use
   SQLite (+ optional sidecar embeddings later). **[INFERRED]**
10. **Cross-encoder reranker as a separate model** — latency and weight;
    prefer RRF alone or a lightweight rerank. **[INFERRED]**
11. **Persistent background consolidation daemon** — do consolidation at
    session boundaries / on-demand instead. **[INFERRED]**
12. **4 parallel retrieval strategies** — semantic + FTS5 keyword + temporal
    is sufficient to start; graph traversal can come later. **[INFERRED]**
13. **Mandatory LLM extraction** — provide heuristic fallback (regex,
    rule-based) before requiring an LLM call. **[INFERRED]**
14. **Enterprise surface** (Oracle, Helm, multi-tenant schema, webhooks,
    Prometheus) — out of scope. **[INFERRED]**