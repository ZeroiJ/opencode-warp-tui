# Phase 8 — Decision Log

> Research only. IDs 8-R1…. Each entry: decision, evidence, alternatives,
> rejected alternatives, compatibility impact, implementation consequence,
> confidence, unresolved questions. Nothing revises D1–D25 or 6A/7A/7C
> decisions unless explicitly stated (none does).

## 8-R1 — Extraction: rules-propose + mandatory confirm; no silent auto-store (A)

- **Decision:** Phase 8 extraction = deterministic rules over user
  messages at session end → quarantine queue → explicit confirm/discard.
  Silent auto-store from any source is prohibited (D-class).
- **Evidence:** D4/D13 (explicit-only V1, ASK gate); Mem0's ADD-only turn
  away from LLM overwrite decisions **[DOCUMENTED]** (`mem0.md` §4.2);
  all surveyed auto-systems need LLM + embeddings + daemons
  **[DOCUMENTED]** (`comparison.md` §9).
- **Alternatives:** explicit-only forever; LLM-assisted now; hybrid now.
- **Rejected:** LLM now (13KB prompts, cost, model-config dependency —
  measure rules precision first); full-auto (false-memory risk, privacy).
- **Compatibility:** additive modules; no schema change except `method`;
  no OpenCode change. **Implementation:** proposal generator + queue +
  suggest/confirm/discard verbs.
- **Confidence:** high. **Unresolved:** rules precision on real
  conversations (measured in implementation fixtures, gate §10).

## 8-R2 — Extraction reads user messages only (A, safety-critical)

- **Decision:** The extractor input set is user-role messages. Never
  model output, tool output, or files.
- **Evidence:** Tool transcripts carry file contents/secrets; project
  files are attacker-controlled; model output confabulates
  **[INFERRED]**; secret-refusal (D23) cannot catch everything, so input
  restriction is the primary guard.
- **Alternatives:** all-message extraction with stronger filters.
- **Rejected:** filters are probabilistic; input restriction is
  structural. Full analysis: `phase8-security-review.md`.
- **Compatibility:** uses existing paged-history read. **Implementation:**
  role filter at extractor input (one predicate + tests).
- **Confidence:** high. **Unresolved:** none.

## 8-R3 — Trigger: session end only (A)

- **Decision:** Proposals generate once per session end (idle/close
  boundary). No per-turn, no background daemon, no periodic timer.
- **Evidence:** Complete history available; off critical path (Letta
  sleep-time scaled down; OpenViking two-phase commit; comparison §13
  "extract at session end and compaction, never per-prompt")
  **[DOCUMENTED]**.
- **Alternatives:** per-turn, daemon, compaction-hook.
- **Rejected:** per-turn (latency + cost per message); daemon (process
  complexity for unbounded gain); compaction-hook (experimental surface).
- **Compatibility:** SSE `session.idle` + summaries already mapped (7B).
- **Confidence:** high. **Unresolved:** exact idle→propose delay
  (implementation tuning, default: on session switch/close + explicit
  `/memory suggest`).

## 8-R4 — Dedup: normalized-text hash on write path (A)

- **Decision:** Canonicalize (lowercase, collapse whitespace, strip
  punctuation/quote-marks) + hash; reject exact-after-normalize
  duplicates at propose time and write time. Keep same-key supersession
  and md5-style content identity (Mem0 pattern).
- **Evidence:** Hermes exact-duplicate refusal **[DOCUMENTED]**
  (`hermes.md` §2.3); Mem0 md5 dedup **[DOCUMENTED]**; Hindsight 0.97
  threshold needs vectors — rejected as overkill **[DOCUMENTED]**.
- **Alternatives:** embedding similarity, LLM dedup judge.
- **Rejected:** both need model/deps for gain unproven at this scale.
- **Compatibility:** pure function in validation layer; no schema change.
- **Confidence:** high. **Unresolved:** canonicalization edge cases
  (Unicode, code spans) — fixture-driven in implementation.

## 8-R5 — Contradictions coexist + surface; no auto-resolution (A-policy, B-detector)

- **Decision:** Same-key update → supersession (exists). Cross-key
  conflicts → both ACTIVE, surfaced in show/list, resolved by user
  update/forget. No machine resolution, no CONFLICT state activation yet.
- **Evidence:** Hindsight refine-with-history + Holographic surfaced-
  heuristic both refuse silent overwrite **[DOCUMENTED]**; every surveyed
  auto-resolver is an LLM judge **[DOCUMENTED]** (`comparison.md` §15).
- **Alternatives:** newest-wins auto-supersede; LLM judge; CONFLICT state.
- **Rejected:** newest-wins destroys history silently; LLM judge needs a
  model; CONFLICT state without a detector is decoration.
- **Compatibility:** zero change (policy statement). A deterministic
  pair-surfacing heuristic (shared rare-term overlap) is B-deferred.
- **Confidence:** high. **Unresolved:** surfacing heuristic evaluation.

## 8-R6 — No stored scores, all fields (A-rejection)

- **Decision:** Reject importance, confidence, trust, evidence-count,
  recency-score, durability, source-reliability as stored fields.
  Reaffirms D3 with Phase 8 evidence.
- **Evidence:** Per-field: no consumer (importance — pinning decides
  order); rot without updater (confidence); needs feedback volume absent
  here (Holographic trust ±0.05/0.10 needs `fact_feedback` traffic
  **[DOCUMENTED]**); computable at read time (recency from timestamps,
  support à la Hindsight proof_count **[DOCUMENTED]**).
- **Alternatives:** Holographic-style trust with manual feedback.
- **Rejected:** feedback source doesn't exist; revisit only with one.
- **Compatibility:** schema unchanged. **Confidence:** high.

## 8-R7 — Retrieval: whole-corpus + lexical ordering; FTS5/embeddings deferred (A/B)

- **Decision:** Keep frozen whole-corpus injection; add std-only lexical
  scorer as ordering input (never a filter). Defer `rusqlite`/FTS5 until
  substring-scan latency is measured bad (threshold: sustained >50 ms at
  real corpus or corpus >10k rows — measure, don't assume). Reject
  embeddings, vector DB, reranker (math: 12k budget ≈ 60–150 rows; no
  recall problem demonstrated).
- **Evidence:** Surveyed retrieval machinery serves 10–1000× scale
  **[DOCUMENTED]**; Hermes/Holographic prove local-deterministic works
  **[DOCUMENTED]**; D8/D24.
- **Alternatives:** rusqlite now; embeddings sidecar now.
- **Rejected:** new dep + daemon/model for unmeasured gain.
- **Compatibility:** scorer is a pure comparator; budget math untouched.
- **Confidence:** high. **Unresolved:** real-corpus latency numbers
  (implementation measures).

## 8-R8 — Session-start preserved; per-turn rejected; first-submit timing specified-not-built (A/B)

- **Decision:** Injection stays session-start frozen (D12/D18 reaffirmed).
  Per-turn retrieval rejected (no OpenCode hook, breaks prefix-cache
  discipline). First-submit injection timing (select after prompt text is
  known, still before first provider turn) specified in retrieval-design
  as config-gated future, default off, not implemented in Phase 8.
- **Evidence:** Context Epoch baseline semantics (6A); no per-turn hook
  on 2.0.8 verified surface (7C).
- **Alternatives:** per-turn now; first-submit now.
- **Rejected/Deferred:** per-turn (no mechanism); first-submit (no
  query-aware demand yet — build the scorer first).
- **Confidence:** high.

## 8-R9 — Provenance: add `method` only (A, single schema change)

- **Decision:** One additive optional field `method: string?`
  (`explicit` | `rule:<name>`); absent reads as `explicit`. Defer
  `confirmed_at`, evidence lists, message-ID links.
- **Evidence:** Needed to distinguish user-stated vs rule-proposed rows
  for audit/poisoning forensics; session_id+quote already locate sources
  (message IDs addressable per 7C) **[VERIFIED]**.
- **Alternatives:** full evidence model now.
- **Rejected:** machinery without a consumer.
- **Compatibility:** forward-compat load rule covers it; no migration.
- **Confidence:** high.

## 8-R10 — No decay/expiry (A-rejection)

- **Decision:** No time/access/confidence decay. Staleness handled by
  supersession + forget + visible timestamps.
- **Evidence:** No demonstrated stale-row harm; Holographic decay
  defaults off **[DOCUMENTED]**; D10.
- **Alternatives:** half-life decay, access-based demotion.
- **Rejected:** unproven benefit, silent behavior change risk.
- **Confidence:** high. **Unresolved:** revisit trigger defined as
  observed stale-row harm (gate §10).

## 8-R11 — No consolidation daemon; on-write deterministic only (A)

- **Decision:** No background process. Consolidation = normalized dedup
  + supersession + proposal-queue draining, all inline and bounded.
- **Evidence:** All surveyed daemons serve scale/complexity we lack
  **[DOCUMENTED]**; daemon = process + scheduling + failure modes for
  unbounded gain.
- **Confidence:** high.

## 8-R12 — Commands: suggest/confirm/discard via existing parser (A); UI in Phase 9 (B)

- **Decision:** Phase 8 extends `Command` enum + in-band replies (D19
  path — intelligence, not UI). Browsing/search rendering stays Phase 9.
- **Evidence:** D19 adapter-side routing exists; parser already owns
  remember/update/forget/list/show **[SOURCE]**.
- **Confidence:** high.

## 8-R13 — Worthiness classes (A-policy)

- **Decision:** KEEP / CONFIRM / DO-NOT-KEEP table is policy Phase 8
  rules encode (full table: intelligence-design §3). Secrets/transients/
  tool-output/model-output never proposed, by construction (8-R2).
- **Confidence:** medium-high (table tuned by implementation fixtures).
- **Unresolved:** fixture measurement may move rows between KEEP and
  CONFIRM.
