# Phase 8 — Memory Intelligence Reconnaissance

> Status: 🔬 RESEARCH ONLY. **Nothing implemented, nothing modified.**
> Evidence: prior decision package (D1–D25, 6A/7A/7C recon), OWT source
> read (`src/backend/memory/`, adapter, mapper), one read-only `/api/info`
> probe (server 2.0.8). Zero sessions created, zero model turns burned —
> every live fact cited here was already verified in Phase 6C/7C.
>
> Labels: **[DOCUMENTED]** prior project docs/source · **[SOURCE]** OWT
> code read this phase · **[VERIFIED]** live probe (6C/7C or this phase)
> · **[INFERRED]** analysis · **[OPEN]** unresolved.
>
> Companion docs: `phase8-decision-log.md` (8-R1…), `phase8-architecture.md`,
> `phase8-retrieval-design.md`, `phase8-intelligence-design.md`,
> `phase8-security-review.md`.

---

## 1. The Phase 8 question, answered up front

"What is the smallest amount of memory intelligence that materially
improves OWT while remaining local, deterministic where possible, safe,
understandable, maintainable, and compatible with OpenCode?"

**Answer: a deterministic, user-gated intelligence layer with no new
dependencies, no embeddings, no LLM requirement, and no OpenCode
changes.** Concretely (§9, gate in decision log):

- Rules-based extraction proposals from **user messages only**, quarantined
  until explicit user confirmation (ASK gate per D13).
- Normalized-text dedup on the write path (exact + canonicalized hash).
- A pure, std-only lexical relevance scorer used for ordering (never for
  silent inclusion/exclusion).
- One additive optional schema field (`method`).
- Everything else in the Phase 8 roadmap list — embeddings, vector DB,
  FTS5-via-new-dep, LLM extraction, rerankers, stored scores, decay,
  background consolidation, per-turn retrieval — is evaluated below and
  **rejected or deferred with reasons**.

## 2. Why reduction wins here (evidence)

1. **Scale**: the 12,000-char default budget holds roughly 60–150
   memories at observed record sizes. Whole-corpus frozen injection
   (D12/D18) is provably sufficient until the corpus persistently exceeds
   budget — a condition no deployment has demonstrated **[INFERRED]**.
   Every surveyed system that runs retrieval machinery (Mem0 hybrid +
   over-fetch 60, Hindsight TEMPR+RRF+CE, OpenViking hierarchical search)
   does so at scales 10–1000× ours **[DOCUMENTED]** (`comparison.md` §12,
   `mem0.md` §4.6, `hindsight.md` §3.2, `openviking.md` §6.5).
2. **Quality anchor**: V1 rows are 100% user-vouched (Model A, D4). The
   dominant failure mode of memory systems — wrongly-extracted rows
   silently injected — is currently *impossible by construction*.
   Any automatic path must preserve that property, which forces
   human-in-the-loop and kills most of the autonomy dividend
   **[INFERRED]** (cf. Mem0's move to ADD-only extraction precisely to
   avoid LLM overwrite decisions **[DOCUMENTED]** `mem0.md` §4.2).
3. **Cost anchor**: Holographic runs fully local with SQLite+FTS5 and
   optional NumPy; Hermes builtin needs no model at all **[DOCUMENTED]**
   (`holographic.md` §7.2, `hermes.md` §2.2). Local-deterministic is a
   proven operating point, not a compromise.
4. **Hindsight's key lesson**: no per-memory confidence score — support
   evidence (`proof_count`) computed at retrieval time instead
   **[DOCUMENTED]** (`hindsight.md` §3.5). Stored scores rot; computed
   signals don't.

## 3. Area findings (8A–8N condensed; full analysis in companion docs)

- **8A extraction**: smallest safe = rules + mandatory confirm, proposed
  at session end from user messages only (details: intelligence-design
  doc). LLM extraction deferred (needs model config + cost + prompts the
  size of Mem0's 13KB **[DOCUMENTED]** `mem0.md` §4.7 — unjustified before
  rules precision is measured).
- **8B worthiness**: KEEP (preferences, stable user facts, project
  decisions/constraints/conventions, tooling prefs) / CONFIRM (anything
  inferred, goals, relationships, model-originated) / DO NOT KEEP
  (secrets, transient state, session-local info, tool output, one-off
  errors). Full table in intelligence-design doc.
- **8C dedup**: normalized-text hash (lowercase, collapse whitespace,
  strip punctuation) + existing same-key supersession + md5-style content
  hash à la Mem0 **[DOCUMENTED]** (`mem0.md` §4.2). Embedding-free:
  **yes, sufficient** — duplicates that survive normalization are
  semantically near-dups requiring judgment, i.e. confirmation-time
  human review, not vectors.
- **8D contradiction**: no auto-resolution (all surveyed auto-resolvers
  are LLM judges: Mem0 update-prompt, OpenViking dedup-LLM
  **[DOCUMENTED]**). Policy: same-key update → supersession (exists);
  cross-key conflicts → coexist + surfaced via `show`/list (Hindsight
  "refine, preserve history" and Holographic "surfaced, not overwritten"
  agree **[DOCUMENTED]**); user resolves with update/forget. A future
  detector is specified as deterministic candidate-pair surfacing
  (shared rare-term overlap), never silent resolution.
- **8E consolidation**: NO background daemon (all surveyed daemons —
  Hindsight observations, Letta sleep-time, OpenViking async commit —
  exist to serve scale we don't have **[DOCUMENTED]**). Lightweight
  deterministic only: dedup-on-write + supersession (exists) + proposal
  queue draining on confirm. LLM consolidation deferred.
- **8F retrieval**: whole-corpus until persistent budget overflow
  (measured, not assumed); lexical scorer ready as ordering signal;
  SQLite FTS5 deferred (needs `rusqlite` — a new dep — for gain unproven
  over substring scan at this scale); embeddings/vector DB/reranker
  rejected with math in retrieval-design doc.
- **8G context signals**: project/cwd (already via project store), prompt
  text (only via first-submit timing — specified, config-gated, default
  off), recency (have). Files-edited, agent, model, tool-activity:
  rejected (complexity + contamination risk). Session-start preserved;
  per-turn rejected (frozen D12/D18 + no OpenCode per-turn hook).
- **8H scores**: all rejected. Per-field verdicts in decision log
  (8-R-series): no consumer (importance), rot (confidence/trust without
  feedback loop — Holographic trust needs `fact_feedback` usage volume
  we don't have **[DOCUMENTED]**), computable at read time instead
  (recency, evidence count à la Hindsight).
- **8I provenance**: current 4 fields + one additive optional `method`
  (`explicit` | `rule:<name>`) — answers "how did this enter" for audit
  and poisoning forensics. `confirmed_at`, evidence lists, message-ID
  links: deferred (session_id + quote already locate the source;
  message IDs verified addressable in 7C but linking every row is
  machinery without a consumer).
- **8J decay**: none. No demonstrated harm from old rows; explicit
  supersession + forget are the mechanisms; Holographic ships decay
  default-off (0=disabled) **[DOCUMENTED]**. Revisit only with evidence
  of stale-row harm.
- **8K commands**: Phase 8 adds `suggest`/`confirm`/`discard` through the
  existing adapter-side parser path (D19 — intelligence, not UI);
  browsing/search UI stays Phase 9.
- **8L/8M privacy & poisoning**: the load-bearing rule is **extraction
  reads user messages only — never model output, tool output, or files**.
  This single rule neutralizes secret exfiltration from tool transcripts,
  instruction-poisoning from project files, and model-originated
  confabulation at the source. Full analysis in security-review doc.
- **8N systems**: delta since Phase 1 — Mem0's ADD-only turn and
  Hindsight's no-confidence-score stance both reinforce our V1 instincts
  **[DOCUMENTED]**; no system surveyed justifies adopting embeddings,
  daemons, or LLM judges at our scale (details per-system in §7).

## 4. Quantitative anchors

| Quantity | Value | Source |
|---|---|---|
| Default budget | 12,000 chars ≈ 60–150 memories | D24 + record sizes [SOURCE] |
| Whole-corpus sufficiency | until persistent budget overflow (never observed) | [INFERRED] |
| Substring scan cost | ~µs–ms at ≤10k records; measure before FTS5 | [INFERRED] |
| Mem0 extraction prompt | ~13KB (local-model-hostile) | [DOCUMENTED] |
| Holographic contradiction | O(n²) guarded at 500 facts | [DOCUMENTED] |
| Hermes budgets | 2200/1375 chars, frozen snapshot | [DOCUMENTED] |
| Hindsight dedup threshold | cosine ≥ 0.97 | [DOCUMENTED] |

## 5. Extension seams (source inspection, all read-only)

- `MemoryApi::{remember, update, forget, list, show, set_pinned,
  ordered_active}` — proposals enter via `remember` with
  `source=user, method=rule:*` after confirm; no API shape change needed
  for storage **[SOURCE]** `api.rs`.
- `MemoryStore` trait + `JsonlStore` — new optional field tolerated on
  load (forward-compat rule) **[SOURCE]** `record.rs:4-8`, `store.rs`.
- `budget::select/ordered_active` — relevance scorer plugs in as an
  ordering comparator; budget math untouched **[SOURCE]** (7A recon).
- `command::{parse, Command}` — `suggest/confirm/discard` extend the
  existing enum + in-band replies **[SOURCE]** `command.rs`.
- Adapter sees session-end (SSE `session.idle`, summaries) and full
  history (paged messages, 7B) — extraction inputs exist without new
  OpenCode surface **[DOCUMENTED]** (7B/7C recon).
- `tombstone` hash — proposals checked against tombstones before
  surfacing (forgotten stays forgotten) **[SOURCE]** (D14 impl).

## 6. Live-probe record (this phase)

- `GET /api/info` → server 2.0.8 (pid 33013). Zero sessions created,
  zero model turns, zero writes. All behavioral evidence reuses 6C/7C
  verification (message-ID addressability, history-after-compact,
  instruction-entry round-trips) **[VERIFIED]**.

## 7. Per-system delta notes (8N update)

- **Mem0**: V3 ADD-only pipeline validates never-overwrite (our D5/D14);
  md5 dedup + scope-injection guard + entity-boost are the portable
  ideas; reject cloud defaults, telemetry, 13KB prompts **[DOCUMENTED]**.
- **Hindsight**: no-confidence-scores + proof-count-at-retrieval-time is
  the strongest external support for 8H rejections; retain/recall/reflect
  is a clean contract shape if we ever split modules; reject Postgres,
  CE-reranker, daemon **[DOCUMENTED]**.
- **Holographic**: asymmetric trust (+0.05/−0.10) is the *only* scoring
  design with a real consumer (retrieval weighting) — but its consumer
  needs feedback volume; adopt the pattern conditionally (deferred until
  a feedback source exists); entity-overlap contradiction heuristic is
  the template for future deterministic surfacing; HRR stays an evaluated
  option, never the primary **[DOCUMENTED]**.
- **Hermes**: frozen snapshot (already our D18), threat-scan at
  write+load (already our D23 + fencing), failure cap, all-or-nothing
  batches, `<memory-context>` fence — V1 already absorbed the portable
  parts; remaining parts need its agent loop **[DOCUMENTED]**.
- **Letta**: tiers map to our Tier-1-always-in-context + future retrieval;
  sleep-time → scaled-down session-end proposals (adopt the timing, not
  the second agent); MemFS git-memory noted as alternative audit model —
  rejected (tombstones + JSONL already cover it, git-sync is scope creep)
  **[DOCUMENTED]**.
- **OpenViking**: progressive L0/L1/L2 is the template for any future
  budget pressure (abstract ≤256 chars); two-phase commit maps to
  propose-then-confirm; reject Doubao coupling, AGPL core, 9-type
  taxonomy **[DOCUMENTED]**.
- **Community**: de-facto SQLite-class storage confirms local-first
  baseline; `experimental.*` hook fragility confirms our adapter-side,
  verified-surface discipline **[DOCUMENTED]** (`opencode-projects.md`).

## 8. Architectural questions — short answers (Q1–Q20)

1. Explicit-only vs automatic? **Explicit + rule-proposed-with-confirm**
   (human stays in the loop; full-auto rejected).
2. Smallest safe extraction? Session-end rules over user messages →
   quarantine → confirm (intelligence-design §2).
3. Deterministic vs LLM vs hybrid? **Deterministic now**; LLM deferred
   until rules precision is measured.
4. Trigger? **Session end** (bounded, complete history, off critical
   path — matches OpenViking two-phase + comparison §13).
5. Session-start retrieval only? **Yes** (frozen preserved).
6. Query-aware where? Nowhere in Phase 8 default; first-submit timing
   specified as config-gated enabler (retrieval-design §5).
7. FTS5 enough? More than enough — deferred until measurement says
   otherwise (no new dep yet).
8/9/10. Embeddings/vector DB/reranker? **No** (math in retrieval-design).
11. Score fields? **No** (8-R series).
12/13. Contradictions? Coexist + surface; supersession on same-key
   update; no silent resolution.
14. Consolidation? Deterministic on-write only; no daemon.
15. Decay? No.
16. Provenance? +`method` field only.
17/18. Poisoning/contamination? User-messages-only extraction +
   quarantine + scope guards (security-review).
19. Strictly user-controlled? Creation (confirm), deletion, scope,
   injection on/off — same as V1 plus proposal triage.
20. Minimum valuable Phase 8? Proposal pipeline + normalized dedup +
   lexical ordering + `method` field + suggest/confirm/discard verbs
   (gate §10).

## 9. Decision framework summary (A/B/C/D)

- **A — REQUIRED**: rule-based proposal pipeline (session-end, user-msgs-
  only, quarantine, confirm/discard); normalized dedup on write;
  lexical relevance scorer (ordering only); `method` field;
  suggest/confirm/discard verbs; tombstone-check on proposals.
- **B — USEFUL BUT DEFER**: LLM-assisted extraction (needs precision
  baseline + model config); SQLite FTS5 (needs scale evidence);
  deterministic contradiction surfacing (needs entity heuristic eval);
  first-submit injection timing (needs query-aware demand);
  `confirmed_at`/message-link provenance (needs consumer).
- **C — NOT JUSTIFIED**: embeddings, vector DB, reranker, stored scores,
  decay/expiry, background daemon, per-turn retrieval, entity graph,
  9-type taxonomy, git-backed memory, progressive L0/L1/L2 (until budget
  pressure).
- **D — DANGEROUS / DO NOT BUILD**: silent auto-store (any source);
  extraction from model/tool output or files; auto-resolution of
  conflicts; silent overwrite (already forbidden); telemetry-tied memory;
  cloud-dependent retrieval.

## 10. Phase 8 Implementation Gate

1. **Implement**: §9-A list only.
2. **Do NOT implement**: §9-C/D lists; anything requiring new deps, LLM,
   embeddings, daemons, OpenCode changes, TUI changes.
3. **Schema changes**: one additive optional field — `method: string?`
   (`explicit` | `rule:<name>`); readers tolerate, writers emit; no
   migration (absent = `explicit` for pre-8 rows).
4. **Dependencies**: none (std + existing serde_json).
5. **OpenCode integration changes**: none (read history via existing
   client; inject via existing entry path; no new routes).
6. **Security guarantees**: user-messages-only extraction; secret-refusal
   on proposals (D23 unchanged); quarantine (proposals never injected);
   tombstone pre-check; scope guards (project rules fire only in their
   root); no file reads by extractor; confirm-before-store.
7. **Tests**: proposal precision/recall on fixture conversations;
   determinism (same history → same proposals); secret/poison fixtures
   never proposed; normalized-dedup vectors; scorer determinism +
   golden orderings; tombstone edge; budget untouched; failure-injection
   (OpenCode/model unavailable → proposals skipped, session unaffected).
8. **Live verification**: throwaway sessions; proposal generation on a
   scripted conversation; confirm→inject round-trip; Mechanical checks
   (204s, markers) — no new model-turn requirements beyond one scripted
   turn if fixtures can't cover it.
9. **Acceptance criteria**: proposal precision ≥ 0.8 on fixture set with
   zero secret/poison proposals; zero schema migration; zero new deps;
   full suite green; determinism tests green; memory-off behavior
   byte-identical.
10. **Deferred to Phase 9+**: browsing/search UI, LLM extraction,
   FTS5/embeddings (on evidence), contradiction surfacing, decay,
   per-turn retrieval, entity graph.
