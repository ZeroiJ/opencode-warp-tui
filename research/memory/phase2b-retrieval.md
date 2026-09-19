# Memory Research Phase 2B — Retrieval Design

> Status: 🔬 RESEARCH ONLY — Phase 2B design output. **Nothing here is
> implemented.** This document answers: what does "retrieval" mean for a
> V1 store whose whole corpus fits the injection budget? It fixes the
> exact ordering algorithm, the budget model (including the hard byte
> ceiling imposed by OpenCode's instruction-entry limit **[VERIFIED]**
> 262,144 bytes), scope resolution, duplicate/staleness/contradiction
> policy, and why V1 needs **no search engine**.
>
> Evidence labels: **[VERIFIED]** live probe · **[DOCUMENTED]** official
> docs/specs · **[INFERRED]** analysis · **[RECOMMENDATION]** Phase 2B
> decision → Phase 5 review.
>
> Companion: `phase2b-storage.md` (schema/lifecycle/tombstones),
> `phase2b-injection.md` (how the block reaches OpenCode),
> `phase2b-decision-report.md`.

---

## 1. What retrieval means in V1

V1 injection is *session-start, whole-corpus-when-in-budget, frozen*
(P2A §15.2, decision D12). At that scale, **retrieval = one deterministic,
budgeted ordered read**: `read ACTIVE, scope-filtered, ordered, truncated
to a char budget`, frozen into a block. There is no query, no ranking, no
Relevance; the *order* is the entire retrieval algorithm, and a hard byte
invariant from the injection boundary caps it.

The same read is also the data source for `list` and `show`; `update`
and `forget` address records **by key/id**, not by search
(§7). On purpose, V1 has exactly one retrieval entry point, so V2's
retriever replaces it behind the same `MemoryStore` query shape
(P2A §15.3, §17).

---

## 2. The exact ordering algorithm (decision D24)

Input: both stores (user scope, project scope). Output: an ordered list of
records to render, then truncate.

```text
1. Filter:   status = ACTIVE                       (SUPERSEDED/DELETED never retrieved)
2. Partition into four tiers, each tier ordered by recency:
     T1  project-scope, pinned=true
     T2  user-scope,    pinned=true
     T3  project-scope, pinned=false
     T4  user-scope,    pinned=false
3. Concatenate: T1 + T2 + T3 + T4
4. Within each tier:
     primary   updated_at DESC
     secondary created_at DESC
     tie-break id ASC                        (fully deterministic, total order)
5. Truncate: walk the ordered list, rendering each record until the
   char budget (§3) is exhausted; drop whole records only — never
   mid-record truncation.
```

Design rationale:

| Choice | Reason |
|---|---|
| Pinned before unpinned | User pinning *is* the V1 importance mechanism (P2A §8:2, D3 and D-superset: "importance is user opinion"). It must outrank recency. **[RECOMMENDATION]** |
| Project tier before user tier within the same pinned-ness | When truncation is needed, task-relevant project facts survive before general user facts. Project context is more specific; matches general→specific? — **no**: this deliberately puts specific first for budget survival. Documented divergence from OpenCode's global-before-project AGENTS.md order because the budget, not the assembly, is what truncates. **[RECOMMENDATION]** |
| Recency = `updated_at` then `created_at` | Recent changes surface first; a memory touched by supersession history still ranks by its own last write. |
| `id ASC` tie-break | `id` embeds `unix_millis_pid_seq` (storage doc §4) so `created_at DESC, id ASC` cannot tie; total deterministic order for tests and for byte-identical snapshots across session starts with identical data. |

**Determinism is a hard requirement** (Phase 2B brief §11): two session
starts with the same store contents must render byte-identical blocks,
so prompt-cache prefixes are stable and tests are reproducible.

---

## 3. Budget model (decision D25)

Three numbers, in order of authority:

| Constraint | Value | Source |
|---|---|---|
| **Hard ceiling** | Instruction-entry value ≤ **262,144 bytes** (measured `maxBytes` on 413) | **[VERIFIED]** live probe, phase2b-storage §2 |
| **Safety limit** | Builder target ≤ **200,000 bytes** of JSON-encoded value | **[RECOMMENDATION]** — margin for JSON escaping (`\uXXXX` up to 6×/char), key/fence overhead, and entry-value metadata |
| **Char budget** | Default **12,000 chars** of memory text, configurable (`memory.budget_chars`, max 30,000) | **[RECOMMENDATION]** |

### 3.1 Encode-bounded truncation algorithm

The char budget selects records; the byte limit validates the result,
**because escaping makes chars ≠ bytes**:

1. Build the ordered list (§2).
2. Add records until the char budget would be exceeded → **drop that
   record** (whole-record granularity) and stop.
3. JSON-encode the block into `{"text": "<block>"}`.
4. If encoded size > 200,000 bytes (only reachable with pathological
   non-ASCII content at a near-limit budget), **re-encode with the last
   record dropped** and repeat until fit.
5. If a *single* record alone exceeds the encoded budget, drop it, warn
   the user in the command log ("memory too large for injection"), and
   still store it (storage ≠ injection).

Why a char budget at all, if bytes are the invariant? Readability and
intent: users reason about text size in characters; the builder
implementation reasons in bytes. Document both, enforce the byte one.
**[RECOMMENDATION]**

### 3.2 What 12,000 chars buys

- V2 raw prompt for a typical session is tens of thousands of tokens; a
  12k-char (≈3k-token) memory block is a few percent of a 200k-token
  window, comfortably inside the compaction envelope
  (opencode compacts against the model window minus headroom;
  **[DOCUMENTED]** specs/v2/session.md).
- Empirical V1 corpus expectation: a few dozen curated memories; 12k chars
  fits the entire corpus in all but pathological cases — which is the
  whole point of P2A §15 ("all, budgeted").

---

## 4. Scope resolution in retrieval

| Question | V1 answer |
|---|---|
| Is user memory injected into *every* project session? | Yes — both stores are always read at session start (adapter-side; no per-project opt-out in V1). **[RECOMMENDATION]** |
| Does project memory override user memory? | **No override exists.** Both blocks are injected, labeled by scope (`phase2b-injection.md` §6); the model sees distinct provenance. Override semantics would *create* the contradiction engine V1 explicitly lacks (§8). |
| Can the two scopes conflict? | Physically impossible to detect (they are separate files, separate scope values); semantically allowed and simply both shown (see contradiction policy, §8). |
| Deduplication across scopes? | **None in V1.** Identical content in user+project stores would be rendered twice (correct behavior: the user deliberately placed it in both). Dedup is a V2 retriever concern. |
| Promotion between scopes? | **Not in V1** — scope is immutable after creation (set at `remember` time; storage doc §4). |
| Project identity | Canonical path of the adapter's project directory; worktrees/renames = separate stores (storage doc §9.2). |

---

## 5. Duplicate handling

| Case | Behavior |
|---|---|
| Same key, same scope, ACTIVE | *Impossible by construction*: same-key remember = supersession (storage doc §12). |
| Same content, different keys, same scope | Allowed; both ACTIVE; both retrieved. Not detectable as "duplicates" without semantics — V1 does not pretend to detect them. |
| Same content in both scopes | Allowed, rendered twice with different scope labels (§4). |
| Superseded records | Excluded from retrieval (filter is `status=ACTIVE`); retained only for `show`. |

The store's *only* automatic treatment of repetition is the tombstone
(blocks future extraction/import of deliberately forgotten content —
storage doc §11). Nothing else dedupes: dedup is semantic work for the V2
retriever/consolidator, explicitly not V1 (P2A §18 removal list).

---

## 6. Stale memory policy (decision)

**V1 does nothing about staleness beyond transparency.** No expiration, no
validity windows, no age-based demotion. Rationale (Phase 2B brief §21):

- P2A §9 already rejects `expires_at` as session-context territory and
  temporal reasoning as V3 work (D10).
- Staleness risk is bounded *by design*: the corpus is user-curated
  (D4), visible (`list` shows timestamps, P2A §15.2), and refreshed at
  every session start (D12). Mid-session staleness is accepted
  (frozen injection — `phase2b-injection.md` §7).
- The only staleness *tools* V1 exposes are the data itself:
  `created_at`/`updated_at` in `list`/`show`, and manual
  update/supersede as the correction path (P2A §13.3).

Age is *displayed*, not *acted on*. A future age-based demotion would be a
retriever-ranking concern (V2+), not a storage field.

---

## 7. Search: what V1 commands actually require (decision)

| Command | Addressing mechanism | Search needed? |
|---|---|---|
| `list` | Whole-corpus ordered read; optional filters `--kind`, `--scope`, `--pinned` | None (linear scan of a tiny file) |
| `show <key\|id>` | Exact key match among ACTIVE, or exact id | None |
| `update <key>` | Exact key | None |
| `forget <key\|id>` | Exact key or exact id | None |
| context construction | §2 ordering | None |

**Therefore V1 needs no substring, token, fuzzy, FTS5, embedding, or
vector search.** The corpus fits the budget, records are addressed by
exact handle, and the injection read is linear. Enumerated and rejected
(Phase 2B brief §12):

| Candidate | V1 role? | Why rejected |
|---|---|---|
| Substring search | ❌ | No command consumes it; forget-by-substring is *deliberately* too dangerous (§7.1) |
| Token/FTS5 | ❌ | Requires the SQLite implementation (storage doc §14) — deferred with search itself |
| Fuzzy | ❌ | Match ambiguity is a footgun for forget; address-by-exact-handle only |
| Embeddings/vector | ❌ | V3 sidecar (Option C) work; no V1 consumer |
| Semantic ranking | ❌ | The whole corpus fits the budget — ranking has nothing to rank against |

### 7.1 Forget by query is rejected for V1 (Phase 2B brief §10)

`forget <query>` (literal text / substring / fuzzy) is **not in V1**.
Deletion must be *deliberate and difficult to trigger accidentally*:

- Literal-text forget risks deleting the wrong memory (multiple matches,
  near-matches).
- Substring forget can silently erase several records at once.
- Fuzzy forget is worse: plausible-looking wrong deletions.

**Safer alternative (adopted):** `forget` accepts **exact key OR exact
id** only. Ambiguity (e.g. the same key exists in both scopes, or an id
looks like a key) → command fails with the matching candidates listed; no
action is taken. The deliberate flow is `list` → pick handle → `forget`.
`--scope` narrows key resolution when both scopes hold the key.
**[RECOMMENDATION]**

---

## 8. Contradiction policy without CONFLICT (Phase 2B brief §20)

V1 has no CONFLICT lifecycle (reserved for V2, D5). How contradictions are
handled:

| Situation | Example | V1 behavior |
|---|---|---|
| Same key, updated | "prefers JS" → "prefers Rust" | Supersession (update). This is *the* contradiction tool V1 has. |
| Same key, same value, restated | idempotent re-`remember` | Also a supersession write (harmless); no content comparison needed — writer semantics are key-based, not value-based. **[RECOMMENDATION]** |
| Different keys, contradictory values | "Prefer Rust." + "Prefer Python." | **Allow both.** Both ACTIVE, both injected, labeled; the user resolves by updating or forgetting one. |
| Project vs user contradiction | user: "Prefer Go" + project: "uses Rust" | Allow both; both shown with distinct scope labels (§4). No cross-scope arbitration. |
| Silent overwrite | engine replaces a row | **Forbidden by design** (storage doc §13 rules; P2A §10.2). |

Why "allow both" is the right V1 default: contradiction *detection* is a
semantic operation (it requires understanding that two statements are the
same proposition — the thing CONFLICT would flag). Without extraction or
semantics, V1 cannot detect it, and **faking detection (e.g. same-key
checking only) would silently discard valid distinct memories.** The
honest policy is: pinning + recency + user curation + explicit
supersession, with contradiction surfacing deferred to V2's CONFLICT
engine. **[RECOMMENDATION]**

---

## 9. Determinism guarantees

1. Same store contents ⇒ byte-identical block (§2 tie-break + §3
   truncation).
2. Byte-identity is the *point*: V2 prompt-prefix cache reuses the
   baseline verbatim within a Context Epoch ([**DOCUMENTED**]
   specs/v2/session.md), and the session id is the provider cache key
   ([**DOCUMENTED**] `runner/llm.ts`). A stable prefix = stable cache.
3. Determinism is unit-tested: any change to ordering/budget code must
   not alter output for unchanged inputs (test §10).

---

## 10. Retrieval test strategy (Phase 5)

| Area | Tests |
|---|---|
| Ordering | Fixture stores asserting exact tier order T1→T4; pin/unpin reorders correctly; `updated_at` primary; `id` tie-break; total order property (no two records tie) |
| Budget | Char-budget selection; whole-record drop at boundary; encode-bounded truncation loop with non-ASCII content (escape expansion); single-record-too-big warn-and-store |
| Scope | user-only, project-only, both; scope label correctness in composition; worktree/path identity via fixture dirs |
| Filters | `status=ACTIVE` only; superseded excluded; deleted excluded (absent) |
| Duplicates | same content different keys both retrieved; cross-scope twins both retrieved, labeled |
| Contradiction | same-key update → one ACTIVE one SUPERSEDED, only ACTIVE retrieved; different-key contradictions both retrieved |
| Determinism | Two independent runs on identical store → identical bytes |
| Forget addressing | exact-key success; exact-id success; ambiguity (both scopes) → error listing candidates, no mutation; no substring/fuzzy path exists |
| Staleness | `list` renders timestamps; no expiration behavior exists (assert nothing expires) |

---

## ARCHITECTURE STATUS

```text
Retrieval architecture: resolved. Implementation NOT authorized.
```

The ordering algorithm, budget arithmetic, scope/duplicate/staleness/
contradiction policies, and the no-search decision are the Phase 5
contract. No retriever, no search engine, no FTS5, no ranking exists or is
authorized by this document.