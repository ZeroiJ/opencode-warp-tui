# §4 — Mem0 (mem0ai)

> Evidence: `mem0/memory/main.py` (3868 lines, downloaded and read),
> `mem0/configs/prompts.py`, `mem0/utils/{factory,entity_extraction,scoring}.py`,
> official docs, paper `arXiv:2504.19413`. Subagent deep-dive notes in
> `/tmp/opencode/subagent-notes/mem0.md`.

---

## 4.1 Identity

| Attribute | Value | Label |
|---|---|---|
| Repo | `mem0ai/mem0` | **[VERIFIED]** |
| License | Apache-2.0 (core) | **[VERIFIED]** |
| Language | Python ≥3.10 (SDK + server); TypeScript plugin | **[VERIFIED]** |
| Version | 2.1.0 | **[VERIFIED]** (pyproject) |
| Stars | ~65.6k ⭐ / 7.7k forks (Sep 2026) | **[VERIFIED]** |
| Paper | arXiv:2504.19413 | **[VERIFIED]** |
| Default stack | OpenAI LLM + OpenAI embedder + Qdrant vector store | **[VERIFIED]** |

## 4.2 V3 ADD-only extraction pipeline

**[VERIFIED]** — as of v2.1.0/main, the automatic `add()` pipeline is
**ADD-only**: memories only accumulate; nothing is deleted/overwritten by
extraction (announced 2026-04).

Flow (`add()` → `_add_to_vector_store()`):

1. **Context gather**: last 10 messages from SQLite history, session scope
   from filters.
2. **Existing-memory retrieval**: `top_k=10` over-fetch of current memories
   (used for dedup + linking only, not as extraction source).
3. **Single LLM extraction call**: `ADDITIVE_EXTRACTION_PROMPT` receives
   new messages, summary, recently-extracted memories (≤20), existing
   memories (≤10), last-k messages (≤20), observation date; returns
   `{"memory": [{"id", "text"}]}`. Prompt rule: *"Your sole operation is
   ADD… do NOT extract new memories from Existing Memories."*
4. **Batch embed** all extracted texts.
5. **md5 hash dedup**: `md5(text)` compared against existing + batch hashes;
   duplicate skipped.
6. **Batch persist** to vector store.
7. **Entity linking** via spaCy extraction.

The old V2 ADD/UPDATE/DELETE LLM decision (`DEFAULT_UPDATE_MEMORY_PROMPT`)
survives only for explicit user `update()` calls, not automatic extraction.
**[VERIFIED]**

## 4.3 Representation

Each memory is a vector-store payload:

| Field | Notes |
|---|---|
| `data` | Memory text |
| `hash` | `md5(text)` for dedup |
| `text_lemmatized` | For BM25 keyword search |
| `created_at` / `updated_at` | ISO 8601 UTC |
| `user_id` / `agent_id` / `run_id` | Scope entities |
| `actor_id` | Named speaker |
| `role` | user/assistant |
| `attributed_to` | Attribution for assistant-generated facts |
| `expiration_date` | Optional YYYY-MM-DD (hidden after) |
| `linked_memory_ids` | Entity-store links |

**History**: SQLite (`~/.mem0/history.db`) — `memory_id`, `old_memory`,
`new_memory`, `event` (ADD/UPDATE/DELETE), timestamps. **[VERIFIED]**

## 4.4 Scopes

- Mandatory scope: at least one of `user_id`/`agent_id`/`run_id` for all
  operations.
- `_IDENTITY_KEYS = {user_id, agent_id, run_id, actor_id}` cannot be set
  via `metadata` — `_strip_identity_keys()` drops them with a warning
  (**scope-injection guard**). **[VERIFIED]**
- Session scope string: `"user_id=…&agent_id=…"` for SQLite history.

**[INFERRED]** This three-axis scoping (user × agent × run) maps cleanly to
this project's needs: user-level (global), project-level (repo), and
session-level (run) memory.

## 4.5 Entities and graph

- spaCy entity extraction: PROPER (0.95), QUOTED (0.75), TOPIC (0.45),
  IDENTIFIER (0.9) confidence tiers. **[VERIFIED]**
- Entity store = separate vector collection `{collection}_entities` with
  `linked_memory_ids`.
- **Entity boosting** in search: matched entities boost linked memories by
  `sim × 0.5 × memory_count_weight`, additive to hybrid score.

## 4.6 Storage / LLM / embedder / reranker factories

- `LlmFactory`, `EmbedderFactory`, `VectorStoreFactory`, `RerankerFactory`
  (all in `mem0/utils/factory.py`). **[VERIFIED]**
- 20+ vector stores (default Qdrant; local: FAISS, Chroma, Qdrant local);
- 11+ embedders (default OpenAI; local: Ollama, HuggingFace, FastEmbed,
  sentence-transformers, LM Studio);
- 20+ LLMs (default OpenAI; local: Ollama, LM Studio, vLLM, LiteLLM);
- 5 rerankers (Cohere, sentence-transformer, zero-entropy, LLM, HuggingFace).
- **Hybrid search**: BM25 (lemmatized) + semantic; `score_and_rank()` =
  `(semantic + bm25 + entity_boost) / max_possible` with sigmoid
  normalization, query-length-adaptive midpoint, threshold gating
  (`default 0.1`), over-fetch `top_k = max(limit*4, 60)`. **[VERIFIED]**

## 4.7 Local-first

**[VERIFIED]** — fully local is supported explicitly:

```python
MemoryConfig(
    llm=LlmConfig(provider="ollama", config={"model": "llama3"}),
    embedder=EmbedderConfig(provider="ollama", config={"model": "nomic-embed-text"}),
    vector_store=VectorStoreConfig(provider="faiss", config={}),
)
```

Caveats: the V3 extraction prompt is ~13KB (needs a capable local LLM);
spaCy models optional extra; defaults still point at OpenAI+Qdrant;
PostHog telemetry is bundled (should be opt-in for local-first). **[VERIFIED]**

## 4.8 OpenCode integration

**[VERIFIED]** — `mem0/integrations/mem0-plugin/.opencode-plugin/`.

- `@mem0/opencode-plugin` — pure TypeScript, no MCP server; backed by the
  Mem0 SDK directly.
- 9 memory tools + 9 skills (`/mem0-remember`, `/mem0-search`,
  `/mem0-dream`, `/mem0-forget`, `/mem0-scope`, …).
- Lifecycle hooks: auto-search on session start and every prompt, error
  memory lookup, compaction context, secret redaction.
- Storage: Mem0 cloud (Qdrant) or self-hosted Docker; local SQLite+usearch
  via the `ZeR020/opencode-mem0` fork.

## 4.9 Assessment for OpenCode — adopt / avoid

### Adopt ([VERIFIED] concepts)

1. **ADD-only extraction discipline** — simpler and more reliable than an
   ADD/UPDATE/DELETE LLM decision loop; deletes stay explicit/user-driven.
2. **`md5(text)` hash dedup** — deterministic, cheap, effective.
3. **Scope-injection guard** (`_IDENTITY_KEYS` + `_strip_identity_keys()`) —
   clean pattern for memory isolation.
4. **Entity extraction + `linked_memory_ids` + entity-boosted retrieval** —
   lightweight graph without a real graph DB.
5. **Rich metadata payloads + SQLite mutation history** — audit trail for
   every ADD/UPDATE/DELETE.
6. **Observation-date vs current-date in extraction prompts** — temporal
   grounding prevents hallucinated timelines.
7. **Factory pattern for LLM/embedder/vector-store** — clean swap boundary.
8. **Hybrid `score_and_rank()` with additive signals** — semantic + BM25 +
   entity boost under one scoring function.

### Avoid / modify

9. **Monolithic `Memory` class** mixing pipeline, storage, telemetry —
   prefer separated components (Extractor → Deduplicator → Storer →
   Retriever → Scorer). **[INFERRED]**
10. **OpenAI SDK as a hard dependency** — lazy-import or HTTP interfaces
    instead. **[INFERRED]**
11. **PostHog telemetry** — must be opt-in for a local-first engine.
    **[INFERRED]**
12. **Qdrant default** — choose SQLite-native first; vector store only as an
    opt-in extension. **[INFERRED]**
13. **13KB extraction prompt** — too heavy for local models; design a
    smaller prompt or heuristic first-pass extraction. **[INFERRED]**