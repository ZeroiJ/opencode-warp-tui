# Phase 8 — Retrieval Design

> Research only. Parent: `phase8-recon.md` §8F/8G, decisions 8-R7/8-R8.

## 1. Sufficiency analysis (when is whole-corpus enough?)

Whole-corpus frozen injection is sufficient while the ACTIVE corpus
serializes under budget with headroom. Anchors: 12,000-char default ≈
60–150 records; truncation order already encodes priority (pinned >
recent). The failure signal is observable, not theoretical: sustained
truncation dropping pinned or recent-project rows across sessions.
**No such signal has been observed; therefore no ranking machinery is
required yet.** Retrieval intelligence in Phase 8 is strictly *ordering
refinement*, never filtering (every ACTIVE row still injects while it
fits — determinism and user-mental-model preserved).

## 2. Lexical scorer specification (the Phase 8 mechanism)

- **Input**: loaded ACTIVE corpus (already in memory) + optional term
  list (empty at session start by default).
- **Tokenize**: lowercase, split on non-alphanumeric, drop tokens < 3
  chars and a small stopword list (fixed in code, tested).
- **Score**: `Σ over distinct matched terms of 1/(1+freq(term))` where
  `freq` counts corpus records containing the term. Deterministic,
  no stored state, O(records × terms).
- **Use**: stable secondary sort key after pinned-tier, before recency
  (exact comparator order specified for implementation; golden tests).
- **Cost**: negligible at ≤10k rows (measure in implementation; gate has
  a 50 ms sustained-latency revisit trigger for FTS5).

## 3. Options evaluated

| Option | Verdict | Reason |
|---|---|---|
| Whole-corpus + D24 (status quo) | KEEP as base | sufficient, deterministic, frozen |
| Lexical scorer (this doc) | ADD (ordering only) | std-only, tested, dormant until useful |
| Substring scan for future search verbs | DEFER to Phase 9 | no search UI yet; scan is the fallback |
| SQLite FTS5 (`rusqlite`) | DEFER (measure first) | new dep; gain unproven over scan at scale |
| BM25 full implementation | DEFER | scorer covers 80%; BM25 when FTS5 lands |
| Embeddings (local or remote) | REJECT (now) | model/dl weight, daemon/service, non-determinism; no recall failure demonstrated |
| Vector DB (any) | REJECT (now) | heaviest option, zero demonstrated need |
| Reranker (CE/LLM) | REJECT | latency + model for unmeasured gain |
| RRF fusion | DEFER | meaningful only with ≥2 signals worth fusing |
| HRR hash vectors | evaluated option, not adopted | unproven quality vs complexity; cited for future |
| Entity-boosted retrieval | DEFER | no entity extraction in V1 (D9) |

## 4. Query-aware retrieval: where it could happen (specified, not built)

Session start has no prompt text, so query-awareness needs the
**first-submit timing**: adapter holds first `submit(text)`, builds the
fenced block with scorer terms from `text`, PUTs entry, then sends the
prompt — still before the first provider turn (Context Epoch compatible
by the same argument as 6C placement). Specified as config-gated
(`retrieval.timing = session-start | first-submit`, default
session-start), implementation deferred until scorer demand or corpus
overflow is observed. No OpenCode mechanism beyond existing PUT is
needed; ordering guarantee is the same code-placement argument as 6C.

## 5. Per-turn retrieval: rejected (reaffirmed)

No verified per-turn injection hook on 2.0.8; would break frozen-snapshot
prefix-cache discipline (D12/D18); every surveyed per-prompt retriever
(Mem0 auto-search every prompt) pays token/latency per message for
recall gains unneeded here.

## 6. Context-signal verdicts (8G)

IN: scope/project root (exists), recency (exists), pinned (exists),
prompt text (only via §4 future). OUT: files-edited, agent, model, tool
activity (contamination risk + complexity; project store already encodes
repo context).
