# Phase 8 — Intelligence Design (extraction, worthiness, lifecycle)

> Research only. Parent: `phase8-recon.md` §8A–8E/8I–8K, decisions
> 8-R1–R5, 8-R9–R13.

## 1. Extraction pipeline (rules-propose-confirm)

```text
trigger: session end (idle/close/explicit suggest)           [8-R3]
input:   user-role messages only (paged history)             [8-R2]
pass 1:  sentence split → candidate filter (length, code,
         URL/key, session-local reference)                   [§2, §4]
pass 2:  rule match (catalog §3 of architecture doc) → typed proposal
screen:  D23 secret patterns → drop with count (never stored, logged)
dedup:   tombstone check → store normalized-hash check → batch check [8-R4]
emit:    quarantine queue file (JSONL, same dir/perms/atomicity as store)
```

Determinism: same history → byte-identical proposals (golden tests).
Cost: zero tokens, microseconds. Failure: any error → no proposals,
session unaffected (memory-never-fatal invariant).

## 2. Worthiness classes (8B policy table)

| Class | Examples | Disposition |
|---|---|---|
| KEEP-propose (preference) | "always write tests first", "I prefer tabs", "never use X" | propose as preference |
| KEEP-propose (fact) | "service uses Postgres", "deploy via nix", "MSRV is 1.75" | propose as fact (project scope iff project-rooted session) |
| CONFIRM-carefully (inferred-ish) | goals ("migrating to Y"), relationships ("X owns auth"), qualified statements ("usually…") | propose with `needs-review` flag; confirm verb shows flag |
| DO NOT KEEP | secrets/keys; error text/stack traces; tool output; code blocks; one-off task state ("restart pod 3"); session-local refs; model-originated claims | negative rules drop silently (counted) |

Scope assignment: project-rooted session + repo-specific nouns →
project; else user. User can re-scope at confirm time (update semantics
already support scope? — confirm verb takes optional scope override;
specify in implementation).

## 3. Lifecycle of a proposal

`proposed → confirmed (remember, method=rule:*) | discarded (no trace)`.
Proposals expire from the queue after 30 days of un-triaged age
(hygiene default; queue is not memory — no tombstone, no injection).
Re-proposal guard: discarded proposal hashes kept in a small
`discarded` set in the queue file so the next session-end doesn't
re-surface the same candidate (bounded at 500, FIFO).

## 4. Contradiction handling in the pipeline

Proposals are checked against ACTIVE rows by normalized hash (dup →
drop) and by same-key (same key → confirm becomes update/supersede
path, shown explicitly). Cross-key semantic conflict: no auto-detect
(8-R5); the confirm verb lists same-scope recent rows for context
(human sees potential conflict at triage time — cheap, effective).

## 5. Consolidation (what runs, when)

Inline only: dedup-on-write, supersession, queue-drain. No daemon, no
timer, no background thread (8-R11). Session-end generation is bounded
by history size already loaded for other purposes where possible;
standalone pass otherwise (still local, still fast).

## 6. Provenance for extracted rows

`session_id` (originating session) + `quote` (user's sentence, ≤256) +
`method: rule:<name>` (8-R9). Confirming session recorded by updating…
nothing extra (update would rewrite provenance — instead, confirm
preserves original provenance; the confirmer's session is the API
caller's context, visible in command logs, not the row). Rationale:
provenance answers "where did the claim come from", not "who clicked".

## 7. Commands (parser-level, D19 path)

- `/memory suggest` — list pending proposals with flags.
- `/memory confirm <n|id> [--scope user|project] [--as fact|preference]` —
  store via remember path.
- `/memory discard <n|id|all>` — drop (discarded-set updated).
Replies reuse in-band Assistant/Error blocks. Rendering/detail is
Phase 9; these verbs are the intelligence contract (8-R12).

## 8. What LLM extraction would require (deferred checklist)

Model config + selection UX; prompt versioning + fixtures; cost/latency
budget; non-determinism acceptance (golden tests become statistical);
local-model minimum capability bar; privacy review for sending history
to remote providers. None of this is Phase 8.
