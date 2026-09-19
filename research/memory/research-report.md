# §38–§42 — Memory Research Phase 1 final report

> Status: 🔬 IN PROGRESS → this report completes the deliverable set.
> Research only; **no implementation** (see §41 stop gate).
> Labels: **[VERIFIED]** primary source / live probe ·
> **[OBSERVED]** seen but not confirmed · **[INFERRED]** analysis.

---

## §38 Summary of findings

1. **OpenCode has no built-in memory** and the live 2.0.8 HTTP/SSE surface
   is rich enough for an external memory engine: session/message APIs,
   active-context, synthetic/generate, instruction entries (experimental,
   verified live), SSE events (incl. compact events mapped in Phase 4),
   VCS diff. **[VERIFIED]**
2. **Injection surfaces rank**: instruction entries (live-verified,
   experimental) ≥ prompt attachments/files (docs-verified) > plugin
   `system.transform` (ecosystem de facto, [`OBSERVED`] absent from
   current official docs) > synthetic messages (endpoint exists, UI cost).
   **Nothing `experimental.*` should be a hard dependency.** **[INFERRED]**
3. **Framework convergence** (§33): bounded in-context curated memory
   (Hermes/Letta) + local SQLite/FTS5 fact store (Holographic + ecosystem)
   + later semantic/graph via sidecar (Hindsight RRF fusion, Mem0 entity
   boost, OpenViking progressive loading, Letta sleep-time, Holographic
   HRR/trust) — no single framework is adoptable wholesale due to
   language/platform/heaviness, but every verified concept maps to a
   lightweight local design. **[INFERRED]**
4. **Key transferable mechanics** (each **[VERIFIED]** in its source):
   - ADD-only extraction + hash dedup (Mem0)
   - Char budgets + frozen prompt snapshot (Hermes)
   - `<memory-context>` fencing + "NOT new user input" note (Hermes)
   - Drift guards + atomic writes + refuse-to-wipe (Hermes)
   - Consolidation failure cap, non-fatal to the turn (Hermes)
   - Trust feedback + contradiction surfacing (Holographic; Hindsight
     refinement-with-history)
   - RRF fusion + token-budgeted recall (Hindsight; engram)
   - Session commit → async extraction → diff audit (OpenViking)
   - Three-tier memory with agent tools (Letta)
   - Scope-injection guards (`_IDENTITY_KEYS`) (Mem0)
5. **Community plugins** prove demand and SQLite-class storage, but lean
   on fragile experimental hooks and are largely low-validation forks —
   reinforcing building our engine adapter-side instead of adopting one.
   **[INFERRED]**

## §39 Evidence & verification status

| Evidence | Type | Status |
|---|---|---|
| `/tmp/opencode/openapi-2.0.8.json` (242 schemas) | Live spec of running 2.0.8 | ✅ saved |
| Instruction entries PUT/GET/DELETE round-trip on ephemeral session | Live probe | ✅ verified, cleaned up |
| Session delete (204); context/synthetic/generate endpoints | Live probe | ✅ verified |
| `hermes_memory_{manager,provider,tool_store}.py` | Source read in full | ✅ captured in §2 |
| Holographic plugin source (`store/retrieval/holographic/__init__`) | Source downloaded + read | ✅ captured in §7 |
| Subagent deep dives (Hindsight/Mem0/Letta/OpenViking/Holographic/OpenCode projects) | Primary-source notes in `/tmp/opencode/subagent-notes/` | ✅ 6/6 completed |
| `experimental.chat.system.transform` in current docs | Doc check | ❌ absent (claims unverified) |
| Community plugin claims (stars, versions, hooks) | GitHub/npm survey | ⚠️ spot-verified; treat counts as time-of-search |
| Embedding/vector quality comparisons (HRR vs trained) | Empirical benchmark | ⚠️ not measured (out of scope) |

## §40 Risks & open questions

1. **Experimental-API churn** — instruction entries and `compacting` carry
   `experimental`; gate and degrade gracefully. **[INFERRED]**
2. **Event coverage** — capture quality depends on which adapter events
   carry text (message parts, tool output, think chunks). Needs a Phase 5
   coverage test (Claim C3). **[INFERRED]**
3. **Extraction without an LLM** — rules-only extraction is weaker; the
   opt-in LLM path needs a configured local/remote model and token budget.
   **[INFERRED]**
4. **Prefix-cache & freshness tension** — frozen per-session Tier 1 is
   cache-stable but may serve stale memory mid-session; mitigation:
   invalidate at compaction boundary. **[INFERRED]**
5. **Secret redaction** — must be applied before storage; patterns v1,
   sampling-based review later. **[INFERRED]**
6. **Scope boundaries** — user vs project vs session overlap can cause
   leakage; use scope-injection guards and provenance. **[INFERRED]**
7. **Open questions for Phase 2**: injection boundary final choice;
   memory dir location (project-local vs user-level); whether an
   OpenCode plugin is ever shipped (vs adapter-only); embeddings
   approach (Ollama sidecar vs HRR vs none); token-counting model
   (tiktoken vs heuristic).

## §41 Next-phase scope and stop gate

**Memory Research Phase 2** (⏳ PLANNED) shall decide: storage engine,
schema freeze, extraction modes, retrieval ranking, scope model,
contradiction policy, privacy controls, injection boundary, and the
Tier 1/2/3 build order — based on §34–§37 of this report.

**Stop gate (now)**: this phase's deliverables are complete once this
report and `phases.md` (state "🔬 Memory Research Phase 1 — IN PROGRESS")
are in place and the Warp checkout is verified clean. **No memory
implementation, no Cargo dependency, no OpenCode modification, no
next-phase work.**

## §42 Final report structure (prescribed)

The **final memory architecture report** (deliverable at the end of
Memory Research Phase 2) must contain, in order:

1. **Executive summary** — decision (A/B/C) and tier plan, one page.
2. **Context & constraints** — OpenCode compatibility requirements,
   local-first, adapter isolation, license, dependency policy.
3. **Verified integration surface** — the exact OpenCode 2.x endpoints and
   hooks with version gates, each tagged **[VERIFIED]** / **[OBSERVED]**
   / **[INFERRED]**; experimental surfaces listed with degradation plan.
4. **Architecture (Mermaid)** — final diagram; must supersede the
   candidate in `architecture.md` §34 with a change log.
5. **Component specifications** — Capturer, Extractor, Store, Retriever,
   Consolidator, Context Builder; interfaces, storage schema, failure
   policy.
6. **Memory model** — types, scopes, fact-vs-inference labeling,
   provenance, trust, contradiction policy.
7. **Tiered implementation plan** — Tier 1 → Tier 2 → Tier 3 with
   acceptance criteria per tier and explicit non-goals.
8. **Privacy & security** — redaction, local-only guarantee, opt-out,
   export/wipe.
9. **Token & context budget** — budgets, prefix-cache strategy, recall
   limits.
10. **OpenCode-version resilience** — what breaks when, feature flags,
    detection at connect.
11. **Test & verification plan** — event coverage tests, retrieval
    evals, upgrade-simulation tests.
12. **Risk register & open questions.**
13. **Decision record** — options A/B/C considered, why the choice was
    made, dissenting views.

This structure guarantees every requirement from `phases.md` (research
questions 1–15) is answered explicitly in one document. Phase 1 stops
here.