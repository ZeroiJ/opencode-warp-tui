# Phase 8 — Architecture

> Research only. How Phase 8 intelligence attaches to the frozen V1
> without changing its semantics. Parent docs: `phase8-recon.md`,
> `phase8-decision-log.md` (8-R1…R13).

## 1. Component map (after Phase 8)

```text
                    ┌─────────────────────────┐
      session end    │  Proposal Generator     │  NEW (8-R1..R3)
  (SSE idle/close   │  rules over user msgs   │  deterministic, std-only
   or explicit)     │  → quarantine queue     │
                    └────────────┬────────────┘
                                 │ confirm / discard
                    ┌────────────▼────────────┐
                    │  Memory API (unchanged) │  remember(source=user,
                    │  + suggest/confirm/    │  method=rule:*) after confirm;
                    │    discard verbs       │  tombstone pre-check (8-R1)
                    └────────────┬────────────┘
                                 │
                    ┌────────────▼────────────┐
                    │  MemoryStore (unchanged │  + normalized-dedup on
                    │  trait + JSONL)        │  write path (8-R4);
                    │                        │  optional `method` field
                    └────────────┬────────────┘
                                 │ ordered_active()
                    ┌────────────▼────────────┐
                    │  Context Builder        │  + lexical scorer as
                    │  (unchanged injection)  │  ordering input (8-R7);
                    │                        │  budget math untouched
                    └─────────────────────────┘
```

No new component owns sessions, prompts, or history. The generator is a
pure function `history → proposals`; the queue is a file beside the store
(same permissions, same atomic-write discipline); verbs reuse the D19
command path.

## 2. Data flow (proposal lifecycle)

```text
session ends ──► collect user-role messages (paged history, existing client)
            ──► rule pass (imperative/declarative patterns, §3 of intelligence doc)
            ──► secret screen (D23, unchanged) ──► tombstone check (D14)
            ──► normalized-dedup vs store + vs batch ──► quarantine file
            ──► user: /memory suggest → confirm <id> → remember() ──► injected next session
                                                        discard <id> → dropped (no tombstone: never stored)
```

Invariants: proposals never enter `memory.jsonl`; never injected;
discard leaves no trace; confirm path is byte-identical to explicit
remember plus `method`.

## 3. Rule catalog (v1, deterministic)

- `imperative-v1`: user sentences matching `^(always|never|prefer|use|avoid|remember)\b` with a durable object; quoted verbatim (≤4096 chars).
- `preference-v1`: "I prefer/like/dislike X" first-person statements.
- `constraint-v1`: "must/must not/only …" project-context statements
  (proposed to project scope only when session has a project root).
- Negative rules (never propose): questions, code blocks/diffs, error
  text, URLs/keys (secret screen), < N content words, session-local
  references ("this error", "right now", "today's deploy" without stable
  anchor).
- Every rule output carries `{text, kind, scope, quote, rule}`; kind
  mapping is fixed (imperative→preference|fact by verb table in
  intelligence doc).

## 4. Lexical scorer (ordering only)

`score(record, query_terms) = matched_distinct_terms` with ties broken by
existing D24 order (stable, deterministic). No IDF store, no weights file:
rarity approximated by `1 / (1 + store_frequency(term))` computed live
over the loaded corpus (bounded, already in memory). Query terms at
Phase 8 default: none (session-start has no prompt) → scorer is dormant
unless first-submit timing is enabled; specified dormancy is intentional
(retrieval-design §5).

## 5. Schema delta

```jsonc
// v:1 record + one optional field:
{ /* ...existing D16 fields... */, "method"?: "explicit" | "rule:<name>" }
```

Absent = `explicit`. Load tolerates (existing forward-compat rule).
No migration, no backfill.

## 6. What does NOT change (frozen reaffirmed)

Store format, atomicity, tombstones, budget math, injection timing +
format + key, session snapshot/resume semantics, capability probing,
secret policy, command routing ownership, TUI code, Backend trait shape
(additive verbs only via existing Command enum path), MockBackend
(exists; parity for new verbs via same in-band path — no new backend
methods needed since verbs ride `submit`).
