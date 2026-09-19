# §7 — Holographic memory (Hermes memory provider)

> Evidence: subagent deep-dive summary
> (`ses_f4a8d1fd2ffeXF64Q7e598BUV9`), plus source downloaded and read:
> `holographic.py` (HRR algebra, 125 lines), `retrieval.py` (hybrid
> FTS5/BM25 + Jaccard + HRR retrieval, 215 lines),
> `__init__.py` (Hermes plugin adapter, 258 lines) from the
> `hermes-plugin-holographic` lineage. All in `/tmp/opencode/`.

---

## 7.1 Identity

| Attribute | Value | Label |
|---|---|---|
| What it is | A **memory provider plugin** for Hermes Agent (not a standalone product) | **[VERIFIED]** |
| Origin | Original plugin by dusterbloom, merged via **PR #2351** into `NousResearch/hermes-agent` | **[VERIFIED]** |
| Standalone repo | `NousResearch/hermes-plugin-holographic` — 1 commit, 0 stars, effectively unmaintained | **[VERIFIED]** |
| Extraction | `iwaan10000vr/vecmemori` repackaged it as a pip package with added neural embeddings | **[OBSERVED]** |
| Language | Python 3 | **[VERIFIED]** |
| License | MIT (plugin); core store is a port from the KIK project (`memory_agent.py` lineage) | **[VERIFIED]** |
| Storage | **SQLite + FTS5** (required) + optional NumPy for HRR retrieval | **[VERIFIED]** |
| Network | Zero network calls; fully local | **[VERIFIED]** |

## 7.2 Component architecture

Four files form the system:

1. `store.py` — SQLite fact store with entity resolution and trust
   scoring (schema-driven fact table: `fact_id, content, category, tags,
   trust_score, retrieval_count, helpful_count, created_at, updated_at`).
2. `retrieval.py` — hybrid retrieval: FTS5 candidates reranked with
   Jaccard similarity and HRR vector similarity, trust-weighted; weighted
   as `fts 0.4 / jaccard 0.3 / hrr 0.3`.
3. `holographic.py` — HRR phase algebra (bind/unbind/bundle in
   angle space, 1024-dim default).
4. `__init__.py` — Hermes `MemoryProvider` adapter exposing
   `fact_store` (9 actions) + `fact_feedback` tools.

**[VERIFIED]** — source docstrings and module contents.

## 7.3 HRR (Holographic Reduced Representations)

**[VERIFIED]** — `holographic.py` (references Plate 1995; Gayler 2004).

- Each concept is a **phase vector** of angles in [0, 2π):
  - `bind` = circular convolution (phase addition)
  - `unbind` = circular correlation (phase subtraction)
  - `bundle` = superposition (circular mean)
- Atoms derive **deterministically from SHA-256** of
  `f"{word}:{i}"` blocks (counter → uint16 → normalize) so representations
  are identical across processes/machines/Python versions — no RNG, no
  model download, no trained embeddings.
- Holds ~O(√dim) bundled items before similarity degrades; cosine
  similarity for matching; role atoms (`__hrr_role_content__`,
  `__hrr_role_entity__`) encode fact structure
  (`bind(role_entity, entity_vec) + bind(role_content, content_vec)`).

**[INFERRED]** This is a **zero-download, zero-training embedding
replacement**: deterministic hashes of words → vectors. Retrieval quality
is empirically unproven against trained embeddings, but the trick is
remarkable for a fully-local, no-model-memory system.

## 7.4 Trust scoring and feedback

**[VERIFIED]** — `__init__.py` + `retrieval.py`.

- Default trust `0.5`; **asymmetric feedback**: +0.05 per "helpful",
  −0.10 per "unhelpful"; clamped to [0, 1].
- Trust is used as a **multiplicative weight** in retrieval ranking
  (conceptually like Hindsight's proof-count boost, but maintained
  per-fact and learned from usage feedback).
- The `fact_feedback` tool trains the store: good facts rise, bad facts
  sink.

## 7.5 Contradiction detection

**[VERIFIED]** — `__init__.py` (contradict action).

- Heuristic: pairs of facts **sharing entities** (Jaccard overlap ≥ 0.3)
  with **low HRR content similarity** get high contradiction scores.
- O(n²) guarded at 500 facts; requires NumPy (falls back to disabled
  without it).

**[INFERRED]** Queryable contradiction → the engine can *surface*
conflicting facts instead of silently overwriting — aligns with
Hindsight's "refine, don't overwrite" philosophy but cheaper (no LLM).

## 7.6 Compositional retrieval (algebraic reasoning)

**[VERIFIED]** — `__init__.py`.

| Action | Meaning |
|---|---|
| `probe` | Entity-specific recall via unbind — "ALL facts about X" |
| `related` | Structural adjacency — what connects to X |
| `reason` | Multi-entity AND via min-similarity — facts touching MULTIPLE entities simultaneously |
| `contradict` | Find facts making conflicting claims (hygiene) |
| `add/search/update/remove/list` | Standard CRUD |

All fall back to FTS5 when NumPy is absent.

**[INFERRED]** The `probe`/`reason` algebra is a preview of "compositional
queries" (the direction OpenViking's intent analysis also points at), but
depends on HRR/entity quality; for Tier 2 we can implement entity-tagged
facts and multi-entity queries without HRR.

## 7.7 Tool surface

- `fact_store` tool: 9 actions (`add`, `search`, `probe`, `related`,
  `reason`, `contradict`, `update`, `remove`, `list`) with
  `content`, `query`, `entity`, `entities[]`, `fact_id`, `category`
  (user_pref/project/tool/general), `tags`, `trust_delta`, `min_trust`,
  `limit`.
- `fact_feedback` tool: `fact_id` + `helpful`/`unhelpful` rating.
- Config under `plugins.hermes-memory-store`: `db_path`
  (`$HERMES_HOME/memory_store.db`), `auto_extract` (false),
  `default_trust` (0.5), `min_trust_threshold` (0.3),
  `temporal_decay_half_life` (0 = disabled), `hrr_dim` (1024),
  `hrr_weight` (0.3).

**[VERIFIED]** — `__init__.py` module docstring + schemas.

## 7.8 Assessment for OpenCode — adopt / avoid

### Adopt ([VERIFIED] concepts)

1. **SQLite + FTS5 as the foundation** — exactly the storage class this
   project wants for Tier 2: single file, zero services, full-text search
   built in.
2. **Asymmetric trust feedback** (+small / −large, clamped) with
   multiplicative retrieval weighting — simple, observable, trains from
   usage.
3. **Contradiction via entity-sharing + low-similarity pairs** — cheap
   heuristic, no LLM, surfaced not overwritten.
4. **Deterministic hash-derived vectors (HRR)** — a fascinating
   zero-download alternative to trained embeddings for a later semantic
   tier; keep as an evaluated option rather than assuming it.
5. **Fact metadata columns** (category/tags/trust/retrieval_count/
   helpful_count/created_at/updated_at) — a direct, copyable fact-schema
   starting point.
6. **Entity-tagged facts + multi-entity queries** — graph-like queries
   without a graph database.

### Avoid / modify

7. **Coupling to the Hermes plugin interface** (`agent.memory_provider`
   imports) — the store/retrieval core is portable; the adapter is not.
   **[VERIFIED]** — imports in `__init__.py`.
8. **HRR as the primary retriever** — empirically weaker than trained
   embeddings for open-domain text; keep FTS5 primary and HRR optional.
   **[INFERRED]**
9. **Regex-based entity extraction** — fragile; prefer a small lexicon +
   patterns or defer entities to the LLM extraction path. **[INFERRED]**