# Memory Research Phase 2A — Memory Model & Minimality Review

> Status: 🔬 COMPLETE — research output only. **Nothing here is implemented.**
> Labels: **[VERIFIED]** primary source / live probe · **[OBSERVED]** seen but
> not confirmed · **[INFERRED]** analysis · **[RECOMMENDATION]** Phase 2A
> decision for Phase 2B/5 review.
>
> This document is a **critical review** of the Phase 1 candidate
> architecture (`architecture.md` §34–§37, `comparison.md` §9–§33,
> `research-report.md` §38–§40). Phase 1's architecture is treated as a
> *candidate to be minimized*, not as truth. For every major component and
> field it asks: **REQUIRED / USEFUL-BUT-NOT-V1 / FUTURE / UNJUSTIFIED**.

---

## 1. Executive summary

Phase 1 proposed a six-component adapter-side memory engine
(Capturer, Extractor, Store, Retriever, Consolidator, Context Builder) with
eight memory types, three scopes, five lifecycle states, five database
tables, and a family of stored scores (trust, confidence, importance,
novelty, proof count, retrieval count). Phase 2A's critical review finds
that **most of that complexity is not justified for a first
implementation**.

The smallest useful model is:

1. **Three components:** a file-backed **Memory Store**, a **Memory API**
   (explicit user commands only), and a **Context Builder** (bounded,
   fenced, labeled injection). The extractor, retriever, consolidator,
   capturer-as-event-listener, and separate Tier-1 layer are all removed
   from V1. They become growth paths behind interfaces, not V1 code.
2. **Two memory `kind`s** (`fact`, `preference`), everything else being a
   tag or future work. **Two scopes** (`user`, `project`) — `session` is
   provenance, not a memory scope. **One source label** (`user` vs
   `inferred`) replaces fact-vs-inference *plus* a confidence score.
3. **No stored scores at all** in V1: no trust, no confidence, no
   importance, no novelty, no proof count, no retrieval count. Provenance
   (session + quote) answers "why does the system believe this?"; lifecycle
   (ACTIVE / SUPERSEDED / DELETED) answers "what changed?"; user pinning
   answers "what matters?".
4. **Minimal lifecycle:** never silently overwrite; an explicit same-key
   update supersedes (old row is retained), an explicit forget hard-deletes
   and tombstones the hash.
5. **V1 ingestion is explicit user command only** (Model A). Automatic
   extraction — rules in V2, LLM-assisted in V3 — is deferred and
   opt-in. Memory creation in V1 always requires an explicit user request.
6. **Storage is a plain file store (JSONL)** — no SQLite, no FTS5, no new
   Cargo dependencies in V1. The `MemoryStore` interface keeps SQLite/FTS5
   as a swappable V2 implementation behind the same boundary.
7. **Option A/B/C re-scored:** A (curated in-context) is a **Strong fit**
   for V1; B (adapter-side engine) is a **Strong fit** as the V2 growth
   path behind the V1 interfaces; C (sidecar) is **Not justified** until
   Tier-3 semantics (embeddings/graph) are wanted.

This results in a deliberately minimal architecture documented in
`minimal-architecture.md`, with every Phase 2A decision recorded in
`decision-log.md`.

---

## 2. Phase 1 assumptions challenged

| Phase 1 assumption | Verdict | Finding |
|---|---|---|
| Six components are needed | **Removed 3½** | V1 keeps Store, Memory API, Context Builder. No Extractor/Retriever/Consolidator; Capturer shrinks to a command handler. §14, §17 |
| Eight memory types | **Collapsed to 2** | `fact`, `preference`. Decision = tag; profile = user-scoped facts; experience/entity/relationship/skill = future. §5 |
| Three scopes (user/project/session) | **Session removed** | Session is a provenance field; memory scopes are user + project. §6 |
| Fact/inference + confidence | **One `source` field** | `user` (vouched) vs `inferred` (machine). No confidence number. §7 |
| Trust/importance/novelty scores | **Removed** | Compute nothing, store nothing; provenance + lifecycle + user pinning. §8 |
| Five temporal timestamps | **Two** | `created_at` (recorded) + mechanical `updated_at`. §9 |
| Five lifecycle states | **Three** | ACTIVE / SUPERSEDED / DELETED; CONFLICT deferred to V2. §10 |
| Provenance / events / entities tables | **Removed** | Provenance = 4 columns on the record. No audit table, no entity table. §11, §13 |
| SQLite + FTS5 in the first build | **Deferred** | Plain file store for V1 behind `MemoryStore`; SQLite/FTS5 re-evaluated in 2B/V2. §17 |
| LLM/rules extraction in V1 | **Deferred** | Explicit commands only (Model A). §14 |
| Tier 1 as a separate always-in-context layer | **Merged** | At V1 scale "Tier 1" *is* the whole corpus; it's storage + injection mode, not a component. §15, §17 |
| Option B primary from the start | **Revised** | A is the correct V1; B is the V2 growth path behind the same interfaces. §17, decision-log D1 |

No Phase 1 assumption survived unchanged. The strongest reversals:
SESSION-as-scope, stored scoring, automatic extraction in V1, and the
five-table SQLite schema.

---

## 3. Memory vs history

### 3.1 The two kinds of data

**Session history** is the raw, timestamped record of what happened in one
conversation: user messages, assistant text, reasoning/thinking, tool
calls and output, shell commands, file edits, errors, transient context.
OpenCode already stores this (session + message APIs, local SQLite; the
adapter already renders it). **[VERIFIED]** History is complete, uncensored
in its raw form, and per-session.

**Durable memory** is the small, curated, cross-session set of statements
that the system carries into *future* turns on purpose: stable facts about
the user, stable facts about the project, and stated preferences. Memory is
what we choose to re-inject; history is what happened once.

### 3.2 Boundary model

```text
SESSION HISTORY
      │  (user issues an explicit memory command, or
      │   a future extractor proposes a candidate)
      ▼
candidate information
      │
      ▼  durability test (§4)
      │
      ▼
DURABLE MEMORY   ──►  injected into future sessions (scoped, budgeted)
```

In V1 the only producer of candidates is the **user issuing a memory
command** — the pipeline above is user-driven end to end.

### 3.3 What is never memory

| Data | Why it stays history |
|---|---|
| Credentials, API keys, passwords, private keys, tokens, personal secrets | Never stored (§12); user command is refused if the content matches secret patterns |
| Transient debugging details ("value was null at line 42") | Useful this session only; fails the transfer test |
| Tool outputs, shell transcripts, diff dumps | Raw evidence, not knowledge; re-derivable from history |
| Reasoning / thinking traces | Process, not product |
| One-off working decisions not anchored to a stable proposition | Session context, dies with the session |
| Temporary exceptions ("use SQLite for this one migration") | Session context; if it becomes durable the user promotes it |

### 3.4 What qualifies as durable knowledge (transfer test)

A statement is a memory candidate when the user (V1) or a future extractor
(V2+) can point at it and say: *"I will want this in a later session, and I
can prove where it came from."* It must pass **all** tests in §4.

---

## 4. Durable-memory criteria (the durability test)

A candidate becomes a memory when it passes every gate:

| # | Test | Question | Fails | Notes |
|---|---|---|---|---|
| T1 | **Transfer** | Is this useful beyond the originating session? | "This session only" | The core question |
| T2 | **Stability** | Is it likely to still hold in the future? | Fast-changing facts | Temporary truths stay in history |
| T3 | **Provenance** | Can we record exactly where it came from? | No attributable source | Session id + quote required (§11) |
| T4 | **Safety** | Storing it risks no secrets or harm? | Credentials/private data | Redaction check at the command boundary |
| T5 | **Explainability** | Can the user be shown it, and why it exists? | Opaque or unshowable content | Visibility is a hard requirement (§16) |

**[RECOMMENDATION]** V1 gates are boolean and self-evident because the
user supplies the candidate. When extraction arrives (V2+), T1/T2 are the
criteria a rules/LLM extractor must apply (with T3–T5 enforced by the
engine). This defines "experience" (§5): an experience is a candidate that
passes T1 but whose natural form is an episodic summary — deferred until
there is an extractor to produce it.

---

## 5. Memory type analysis

Phase 1 proposed: profile, preference, project fact, decision, experience,
entity, relationship, skill (§14 of `comparison.md`). **[INFERRED]**

| Candidate type | Keep? | Why? | V1/V2/V3 | Alternative representation |
|---|---|---|---|---|
| profile | ❌ | Not a type: a bundle of user-scoped facts/preferences | — | `scope=user` facts + preferences |
| preference | ✅ | Stable, few, user-owned; distinct lifecycle (updates, not conflicts) | V1 | `kind=preference` |
| project fact | ✅ | Core value: architecture decisions, conventions, repo facts | V1 | `kind=fact` `scope=project` |
| decision | ⚠️ | A decision *is* a fact of choice with provenance | V1 as tag | `kind=fact` + tag `decision`; provenance holds session/quote |
| experience | ❌ V1 | Episodic summary; needs extraction to exist at all | V2/V3 | produced by V2 extractor as `source=inferred` summary rows |
| entity | ❌ V1 | Name as metadata; text search covers V1 recall | V3 | entity index/graph from memory text (V3) |
| relationship | ❌ | Graph structure; FTS-textable in the meantime | V3 | edges derived in V3 from text + entities |
| skill | ❌ | Procedural knowledge ≠ retrievable statements; lives in project docs/AGENTS.md | — | never a memory row |

### Findings

1. **`decision` is not a subsystem.** A decision is a fact whose provenance
   (session, quote, context) records the choice. Its lifecycle is that of a
   fact: it can be superseded by a later decision on the same proposition.
   There is no separate decision store, no decision-specific retrieval, and
   no decision-specific lifecycle. **[RECOMMENDATION]**
2. **`preference` is worth one distinct kind.** "User prefers Rust" has
   different update semantics than "Project uses PostgreSQL": a preference
   change is an *update* (supersession), never a conflict; preferences are
   permanently user-scoped. That single behavioral difference justifies one
   `kind` value — nothing more. **[RECOMMENDATION]**
3. **`entity` should not exist in V1.** "PostgreSQL" in "project uses
   PostgreSQL" is searchable by plain text. Entity extraction, indices,
   linking and graphs are exactly the kind of machinery that a small corpus
   does not need. Phase 1's candidate `entities` table is dropped. **[RECOMMENDATION]**
4. **`skill` is not a memory.** "Always run `cargo fmt` before committing"
   is procedure; it belongs in project-level configuration/docs (the
   existing AGENTS.md mechanism), not in a retrievable statement store.
   **[INFERRED]**

**[RECOMMENDATION]** V1 record `kind` = `fact | preference`. Optional
free-form `tags` (e.g. `decision`, `convention`) stay in the record shape
for display but drive no behavior in V1.

---

## 6. Scope analysis

Phase 1 proposed USER / PROJECT / SESSION. **[INFERRED]**

### 6.1 USER scope — REQUIRED

Stable preferences ("prefers Rust over Go for new services"), long-term
profile facts, communication preferences ("wants concise replies"). Stored
in a user-level directory, shared deliberately across projects. Few rows,
long-lived, monotonic growth.

### 6.2 PROJECT scope — REQUIRED

Architecture decisions, project conventions, repo-specific facts,
technology choices. Stored **in a per-repo location** (the project store
lives inside the repo directory — final path decided in 2B), which makes
isolation structural: each repository has its own store file. **[RECOMMENDATION]**

### 6.3 SESSION scope — NOT a memory scope

| Option | Verdict |
|---|---|
| A. First-class memory scope | ❌ |
| B. Temporary working memory | ⏳ later, as pinned *session notes*, not durable memory |
| C. Represented via OpenCode history | ✅ this is what history is for |
| D. Supported later, excluded from V1 | ✅ same as B |

Sessions appear in memory **only as provenance** (`session_id`,
`source_ref`, `quote`). "What was decided in this session" is either a
project-scoped fact (when durable) or history (when not). **[RECOMMENDATION]**
This removes the need for "experience" as a session-scoped type (§5) and
for a session scope filter everywhere.

### 6.4 What belongs where

| Content | Scope |
|---|---|
| Coding preferences (languages, style) | user |
| Communication preferences | user |
| Long-term profile facts | user |
| Architecture decisions | project |
| Project conventions | project |
| Repo-specific facts / tech choices | project |
| Per-session working state | *not memory* — OpenCode history |

---

## 7. Fact vs inference

### 7.1 Binary classification is not enough — but it should also not become a scoring system

There is exactly one axis that must survive into the engine:
**who vouches for this statement?**

- `source=user` — the user explicitly stated it (or confirmed it). The
  quote in provenance is the user's own words. Used as authority.
- `source=inferred` — the system derived it (V2+ extraction). Marked and
  treated as provisional.

**[RECOMMENDATION]** A single `source ∈ {user, inferred}` field replaces
Phase 1's `fact_inference` column *plus* a confidence score. In V1 every
stored memory is `source=user` by construction (ingestion is explicit
user command), so the invariant "everything is labeled" is satisfied at
zero cost and the label becomes meaningful when extraction arrives.

### 7.2 The questions, answered

1. **Is binary classification enough?** Yes — when combined with the
   provenance quote. The quote is the evidence; the label is the epistemic
   status. **[RECOMMENDATION]**
2. **Do we need confidence?** No, not in V1. "Confidence" would be a number
   nobody updates and nothing acts on in a user-curated corpus. Provenance
   gives the user the raw material to judge; if the user vets it, it's
   `user`; if not, it isn't stored at all in V1. **[RECOMMENDATION]**
3. **Do we need source reliability?** Not as a field. Reliability is
   encoded in `source` (user > inferred) and in the quote. **[RECOMMENDATION]**
4. **Can provenance replace confidence?** Yes for V1. The quote answers
   *why the system believes this*; the user decides whether to keep it.
   **[INFERRED]**
5. **Should inferred conclusions be stored at all?** Yes, but only from
   V2 onward, always labeled `inferred`, always provenance-linked, and
   never presented as user-stated. **[RECOMMENDATION]**
6. **Should inferred memories expire faster?** No expiry mechanism in V1
   (§9); staleness is handled by lifecycle (supersession) and user
   deletion. **[RECOMMENDATION]**
7. **Should inferred memories require explicit confirmation?** In V1 the
   question is moot (no inferences). In V2, confirmation is the `user`-vouch
   path: an inferred row may be promoted to `user` by an explicit user
   action. **[RECOMMENDATION]**
8. **Can the model use an inference as a fact?** Never silently. The label
   travels with the row and is rendered in the injected block
   (§16). **[RECOMMENDATION]**

### 7.3 Minimal semantic model

```text
memory = { text, kind, scope, source, key?, tags?,
           session_id?, quote?, status, created_at }
```

No confidence, no evidence count, no reliability. The *semantic* content is
`text`; everything else is bookkeeping that supports explainability,
scoping, and lifecycle.

---

## 8. Trust, confidence, importance, novelty — decision matrix

Phase 1 floated trust, confidence, importance, novelty, durability,
evidence count, retrieval count. **[INFERRED]**

| Concept | Meaning | Who assigns | When it changes | Action it affects | Computable? | V1? | Replaced by |
|---|---|---|---|---|---|---|---|
| trust | credibility of a row | system heuristics | feedback | retrieval ranking | partially (feedback math) | ❌ | provenance + lifecycle |
| confidence | system's certainty | extractor | re-extraction | ranking / surfacing | needs scoring infra | ❌ | `source` label + quote |
| importance | how much to show | system or user | ? | injection budget | no | ❌ | user pinning + budget order |
| novelty | newness vs known context | retriever | per query | dedup | yes, at query time | ❌ | nothing (no retriever in V1) |
| durability | persistence value | extractor | ? | ingestion gate | no | ❌ | durability test §4 |
| evidence count | number of proofs | system | new sources | ranking | yes, = len(evidence) | ❌ | provenance refs (count on demand, V2) |
| retrieval count | usage | retriever | per retrieval | future learning | yes | ❌ | nothing (analytics) |

### Findings

1. **The danger Phase 2A names is real**: a row with
   `trust=0.73, confidence=0.81, importance=0.64, novelty=0.41` carries no
   explainable meaning, so nothing can safely act on it. No Phase 1 concept
   has a well-defined action that V1 needs.
2. **Importance is user opinion.** At V1 scale the honest importance
   mechanism is the user pinning or ordering rows, and the injection budget
   truncating in that order (§16). A stored `importance` float is
   strictly worse. **[RECOMMENDATION]**
3. **Trust reappears only with extraction.** When V2 rules/LLM extraction
   starts writing rows, a Holographic-style feedback signal
   (trust moved by user feedback, **[VERIFIED]** in `holographic.md`) is a
   credible future mechanism — behind the store interface, opt-in, and none
   of it exists in V1. **[RECOMMENDATION]**
4. **Novelty is a retrieval-time computation**, never a stored field, and
   there is no retrieval in V1.

**[RECOMMENDATION]** V1 record carries **zero score fields**. Explainability
comes from provenance; ordering comes from user pinning + recency; the
lifecycle handles change. If a future phase needs scoring, it is added
behind the store interface without touching V1 semantics.

---

## 9. Temporal model

Phase 1 listed created_at / occurred_at / learned_at / updated_at /
superseded_at / expires_at. **[INFERRED]**

### 9.1 Minimum

| Field | Needed | Role |
|---|---|---|
| `created_at` | ✅ REQUIRED | recorded by the store; ordering + UI display |
| `updated_at` | ✅ mechanical | maintenance; status transitions |
| `occurred_at` | ❌ V1 | only meaningful when an extractor observes ≠ stores; V2+ |
| `learned_at` | ❌ V1 | ditto |
| `superseded_at` | ⚠️ | derivable from `status` + `updated_at`; no separate column |
| `expires_at` | ❌ | temporary truths are session context, not memory; no expiry engine in V1 |

### 9.2 Current vs historical facts

"User used Python in 2025" and "User now prefers Rust" are **different
propositions** and both can be active independently. Supersession (§10)
only applies when the *same proposition* is restated — identity in V1 is
the user-provided `key` (or exact content for keyless rows). Truth windows
are expressed in the text itself ("as of 2025", "currently"), or through
lifecycle (old proposition superseded by the new one). No temporal
reasoning is required to represent either case. **[RECOMMENDATION]**

### 9.3 Verdict

> **We do not need temporal reasoning in V1.** Provenance (session id +
> quote + created_at) plus lifecycle (status) plus user-authored text is
> sufficient. Temporal reasoning is V3 work (Phase 8 territory), not V1.

---

## 10. Contradictions and supersession

### 10.1 The five situations

| Situation | Example | Classification | V1 behavior |
|---|---|---|---|
| Update | "prefers JS" → "now prefers Rust" (same key) | supersession | old row → SUPERSEDED; new row ACTIVE; both visible |
| Contradiction | two active rows, same proposition, no update | conflict | V2: keep both + flag CONFLICT; V1: unreachable (no machine extraction) |
| Ambiguity | rows that merely *look* related | not a system problem | user resolves via commands |
| Temporary exception | "SQLite for this one migration" | session context | never stored |
| Silent overwrite | engine replaces a row automatically | **never allowed** | forbidden by design (§10.2) |

### 10.2 Silent overwrite: never

All surveyed systems that handle updates carefully retain history
(Mem0 ADD-only [**[VERIFIED]**], Holographic surfaced conflicts
[**[VERIFIED]**], Hindsight refine-with-history [**[VERIFIED]**], Hermes
replace-with-failure-cap [**[VERIFIED]**] — `comparison.md` §15). V1
follows: **a stored memory is never silently destroyed or replaced.** An
explicit same-key restatement supersedes (the old row is retained and
marked); an explicit forget deletes (hard delete + tombstone, §13).
Consistency cost is trivial because supersession needs no automatic
detection — the *user* provides identity via the key. **[RECOMMENDATION]**

### 10.3 Minimal lifecycle

```text
ACTIVE ──(explicit same-key update)──► SUPERSEDED (kept, linked, excluded from injection)
ACTIVE ──(explicit forget)──────────► DELETED (hard delete + hash tombstone)
ACTIVE ──(V2 machine conflict)──────► CONFLICT (kept both, surfaced)   [V2+]
```

**Do we need all five Phase 1 states?** No. ARCHIVED is dropped (forget
covers it; nothing else needs a parked state in V1). CONFLICT is defined in
the model but unreachable in V1 (no extraction ⇒ the engine never detects a
conflict on its own); it is implemented with V2 rules extraction.
**[RECOMMENDATION]**

### 10.4 What the states mean for injection

Only ACTIVE rows are injected. SUPERSEDED rows are retained for
`/memory show` history and explainability. DELETED rows (tombstoned) are
gone from all views and can never be re-extracted or re-imported (§13).

---

## 11. Provenance model

### 11.1 The requirement

Every memory must answer: **"Why does the system believe this?"** — from
the record itself, without opening OpenCode's entire conversation history.

### 11.2 Minimum: four fields on the record (no table)

| Field | V1 | Why |
|---|---|---|
| `session_id` | ✅ required | which conversation produced it (adapter knows the active session id at command time — no new OpenCode surface needed **[VERIFIED]**) |
| `source_ref` | ✅ required | best-effort turn/message reference within that session ("msg #3", step id) — recorded when identifiable |
| `quote` (≤256 chars) | ✅ required | the user's own words (for `user` source) — the evidence itself |
| `created_at` | ✅ required | when captured |

### 11.3 Rejected provenance extras (V1)

| Extra | Verdict | Reason |
|---|---|---|
| source type enum | ❌ | `source` (user/inferred) already covers it |
| source file | ❌ V1 | nothing links memories to files yet (V3 interest) |
| extraction method / model used | ❌ V1 | extraction is "user command" in V1 — constant; V2+ may add a method field on inferred rows |
| user confirmation flag | ❌ | confirmation *is* the `user` source in V1, or a supersession event |

### 11.4 Memory → evidence → source

```text
memory (statement)
   ↓
source = user | inferred          (epistemic status)
   ↓
session_id + source_ref + quote   (the evidence, self-contained, ≤256 chars)
```

No duplication of whole conversations; the quote is the evidence. Deleting
a memory deletes its evidence with it (§13). No `provenance` table, no
`events`/audit table — provenance is 1:1 with the row and lives on it.
**[RECOMMENDATION]**

---

## 12. Retention policy

### 12.1 Creation policy

In V1 memory creation **requires an explicit user request** for every
category:

| Category | Policy |
|---|---|
| Coding / communication preferences | AUTO-STORE on explicit command |
| Project facts / conventions / choices | AUTO-STORE on explicit command |
| Architecture decisions | AUTO-STORE on explicit command |
| Explicit corrections | AUTO-STORE as supersession (same key) |
| Repeated behavior / recurring patterns | NEVER-STORE in V1 (needs extraction → V2) |
| Temporary debugging details | NEVER-STORE (history) |
| Credentials, API keys, passwords, private keys, personal secrets | NEVER-STORE — command refused on secret-pattern match |

### 12.2 ASK appears with extraction

When rules extraction lands (V2), low-risk candidates ("user said 'always X'")
go through an **ASK** gate: propose, user confirms, only then store as
`source=user`. LLM extraction (V3) proposes `source=inferred` rows that are
labeled, listed, and deletable — never injected as user-stated. **[RECOMMENDATION]**

### 12.3 Secret refusal

The memory API checks command content against a small secrecy pattern list
(`password`, `secret`, `token`, `apikey`/`api key`, `private key`, …) and
**refuses the store with a visible message** — storage is local and
user-controlled, so safety is enforced by refusal rather than redaction
mangling. **[RECOMMENDATION]**

---

## 13. Deletion and correction model

Deletion is a first-class architectural requirement; the simplest model
that is actually trustworthy is:

### 13.1 Forget = hard delete + tombstone

`/memory forget <key|query>`:

1. The row and its provenance fields are **hard-deleted** (removed from
   the store file).
2. A **tombstone** row `{content_hash, deleted_at}` is written so that any
   future extraction/import pass (V2 dedup, re-import, rules extraction)
   will not resurrect the same statement. **[RECOMMENDATION]**
3. Nothing else changes. OpenCode session history is untouched — a deleted
   memory can still be read from history, but **it is never re-injected
   and never re-extracted**.

### 13.2 Answers

1. **What exactly disappears?** The memory row, its quote/source fields;
   the tombstone retains only the hash (not content).
2. **Can deleted info reappear through session history?** It remains *in
   history* (OpenCode's own data, untouched), but never re-enters the
   memory store: tombstone blocks extraction (V2) and the row is excluded
   from injection. This is the honest boundary: memory is what we inject,
   history is what happened. **[INFERRED]**
3. **Should audit logs retain evidence it existed?** No in V1 — local,
   private, user-determined; no content-bearing audit trail. An opt-in
   event log (deletion times/keys only) can come in V2. **[RECOMMENDATION]**
4. **What does "permanently deleted" mean locally?** The row is removed
   from the store file; the tombstone hash prevents re-adoption. For a
   local single-user store this is as permanent as it gets; the user owns
   the file. **[INFERRED]**
5. **Derived memories?** V1 has none. V2 rule: deleting a row also
   deletes `source=inferred` rows that cite only it as basis (bubbling);
   rows with other surviving evidence keep a resolved source_ref note.
   Designed now, implemented with extraction. **[RECOMMENDATION]**

### 13.3 Correction

Correction is **not deletion** — it is supersession: `/memory remember <key>
<new text>` on an existing key marks the old row SUPERSEDED and writes the
new ACTIVE row. Both remain visible in `/memory show`. Never silent
overwrite (§10.2). **[RECOMMENDATION]**

---

## 14. Extraction minimality

### 14.1 Model comparison

| Model | Quality | Cost | Latency | Privacy | Failure modes | Complexity | OpenCode compat | User control |
|---|---|---|---|---|---|---|---|---|
| **A. Explicit user-controlled** | perfect (user-curated) | zero | zero | perfect | none that corrupt memory | minimal | none needed (adapter command) | complete |
| B. Rules-based automatic | low–mixed (noise) | low | low | needs redaction | false positives pollute context | low–med | needs event coverage (C3) | partial |
| C. LLM-assisted | high | tokens + model config | adds at session end | model sees data | wrong extraction is silent pollution | medium-high | needs model wiring | partial |
| D. Hybrid | high | grows | grows | layered | composite | high | layered | layered |

### 14.2 Verdict

> **We do not need automatic extraction at the beginning.**

V1 uses **Model A exclusively**: the user types `/memory remember`, and
that is the only ingestion path. Reasons:

1. **Noise is worse than no memory.** Wrongly-extracted, wrongly-labeled
   rows get injected into future sessions and erode trust in the whole
   system. With a user-curated corpus, every injected row was vetted by
   the only authority that matters.
2. **Cost/quality for rules is poor** for the high-value kinds
   (preferences, decisions need context rules cannot capture).
3. **LLM extraction needs a configured model**, which the adapter cannot
   assume (it is backend-agnostic; no provider is guaranteed).
4. **Privacy** — explicit capture is fully transparent by construction.
5. **OpenCode compatibility** — Model A needs zero new OpenCode surface;
   event-driven capture depends on unverified event coverage (Claim C3 in
   `architecture.md` §34.4, **[INFERRED]**).

**[RECOMMENDATION]** V1 = Model A. V2 = add narrow rules extraction with an
ASK gate + event-coverage verification. V3 = opt-in LLM extraction behind a
config flag (local model preferred). The retrieval interface implemented in
V1 (budgeted read of ACTIVE rows) is the same front door V2/V3
extractors feed.

---

## 15. Injection minimality

### 15.1 What should be injected?

With V1's bounded, user-curated corpus, the honest answer is: **the whole
corpus, scoped and budgeted** — no retrieval ranking is needed at V1 scale.

- project-scope ACTIVE rows (in the project store)
- user-scope ACTIVE rows (in the user store)
- truncated to a hard char budget, in **user-pinned-then-recency order**
- rendered as a **fenced, labeled block** (`<memory-context>` … `NOT new
  user input` — Hermes pattern [**[VERIFIED]** `hermes.md`]), each row
  tagged `[fact]`/`[preference]`, `[user]`, scope, and date.

All memories ≠ summaries ≠ top-K: at V1 scale "all, budgeted" is both
simpler and more honest than a retriever that would rank only a handful of
rows.

### 15.2 Session-start vs per-turn

- **Session-start injection, frozen for the session** (prefix-cache
  stability, Hermes snapshot pattern [**[VERIFIED]**]): V1.
- **Per-turn retrieval / retrieval-only-when-relevant**: V2+, when a
  corpus big enough to overflow the budget arrives with extraction.
- **Stale-memory risk**: bounded by corpus size, user visibility
  (`/memory list`), and refresh at each session start; mid-session
  staleness is accepted (no mid-session re-injection in V1).

### 15.3 Is Tier 1 "always in context" justified?

Yes, but **not as a separate component**. "Tier 1" at V1 scale *is* the
whole store: the entire corpus is small, curated, and injected. The Phase 1
split (Tier 1 files + Tier 2 SQLite engine) collapses into one store whose
injection mode happens to be always-in-context. When the corpus grows
(V2+), retrieval takes over injection and "Tier 2" replaces "Tier 1"
seamlessly behind the same Context Builder. **[RECOMMENDATION]**

### 15.4 Injection boundary

Carried from Phase 1, unchanged: instruction entries (live-verified,
experimental) ≥ prompt attachments (docs-verified) > plugin
`system.transform` ([**OBSERVED**], absent from current docs) > synthetic
message — chosen and version-gated in Phase 2B (§20/Q4). Nothing
`experimental.*` is a hard dependency.

---

## 16. Failure model

Memory is an **optional subsystem**; the invariant is:

```text
Memory failure ──► normal OpenCode operation continues
```

| Failure | V1 behavior |
|---|---|
| Store unavailable / locked / disk error | memory commands report a visible error; turns proceed normally — memory is never on the turn's critical path |
| Store file corrupt / malformed row | skip bad rows, log warning, serve the rest; never crash |
| Memory API command malformed | reject with usage message; no state change |
| Injection boundary missing / version drift | feature gate off ⇒ no injection, log; turns unchanged |
| Secret-pattern match | refuse the store, visible message (§12) |
| Memory content is prompt-injection-like | memory is **data, not instructions**: fenced block + "not new user input" label + never executed as tool/command input (Hermes discipline [**[VERIFIED]**]) |
| Store read fails at injection time | inject nothing; turns unchanged |

Guarantees: no memory path blocks, no memory path crashes, no memory path
silently corrupts context. All memory I/O is adapter-side, local, and
off the streaming/rendering path.

---

## 17. Minimum architecture

Phase 2A's minimum architecture is deliberately small (details + Mermaid in
`minimal-architecture.md`):

```text
OpenCode Adapter  (existing Backend boundary, unchanged)
      │  /memory commands (explicit user request)
      ▼
Memory API  ──►  MemoryStore (file-backed; user + project scopes)
      ▲                    │
      │                    ▼  (ACTIVE, scoped, ordered)
      │              Context Builder  ── fenced + labeled + budgeted
      │                    │       block
      └────────  version-gated injection boundary → OpenCode session
```

**Kept:** MemoryStore (with its `MemoryStore` trait), Memory API (command
handler), Context Builder.

**Removed from V1:** Capturer-as-event-listener (commands replace it),
Extractor, Retriever, Consolidator, entity subsystem, separate Tier 1
layer, five-table schema, all score fields, session scope, provenance/audit
tables.

**Extension seams (designed now, unused in V1):**
- `MemoryStore` trait → SQLite+FTS5 implementation swap (2B/V2);
  takes `{scope,status}` queries and returns ordered rows — the exact
  shape an FTS5-backed implementation and a future retriever need.
- Injection boundary behind config + version gate (2B).
- Evidentiary `hash`/tombstones → dedup once extraction exists (V2).
- Storage location scheme → project-local + user-local (2B determines the
  exact paths and git-ignore politics).

---

## 18. What was removed from Phase 1

| Removed | Why |
|---|---|
| Capturer component | No event-driven capture in V1; the Memory API command handler is the only ingest path (§14, decision-log D4) |
| Extractor component | Explicit-only ingestion (Model A); extraction returns with V2 rules / V3 LLM behind the same interface (§14) |
| Retriever component | Corpus fits the budget; injection reads ACTIVE rows in pin/recency order — ranking is V2 (§15) |
| Consolidator component | Dedup/conflict machinery needs extraction to matter; supersession is a store op on user command (§10, §14) |
| Separate Tier 1 layer | "Tier 1" = the whole corpus at V1 scale; one store, one injection mode (§15, D1/D11) |
| Entity / relationship subsystem | Text search covers V1; graph is V3 (§5) |
| Session scope | Provenance not memory (§6, D2) |
| Trust / confidence / importance / novelty / proof / retrieval scores | No explainable action consumes them in V1; provenance + pinning + lifecycle cover the real needs (§8, D3) |
| `fact_inference` + confidence | One `source ∈ {user,inferred}` field (§7) |
| Five lifecycle states | ACTIVE/SUPERSEDED/DELETED now; CONFLICT reserved; ARCHIVED dropped (§10, D5) |
| `occurred_at` / `learned_at` / `expires_at` / `superseded_at` | created_at + updated_at + status suffice (§9, D6) |
| Provenance table, events/audit table | Provenance is 4 columns on the row; no content audit trail in V1 (§11, §13, D7) |
| SQLite + FTS5 in the first build | File store behind the trait; SQLite re-evaluated in 2B/V2 when corpus + extraction justify it (§17, D8) |
| Automatic extraction (rules/LLM) | V1 is explicit user command only (§14, D4) |

---

## 19. What remains future work

- **V2 (after 2B):** rules extraction with ASK gate; SQLite+FTS5
  implementation (or retention of file store at larger scale); per-turn
  retrieval; `inferred` rows + `CONFLICT` lifecycle; event-coverage
  verification (Claim C3); storage-location freeze and migration path.
- **V3 / Phase 8:** opt-in LLM extraction (local first); embeddings via
  sidecar (Option C) with RRF fusion; entity graph + relationship edges;
  temporal reasoning; optional trust feedback (Holographic-style,
  [**[VERIFIED]**]); background consolidation.
- **Phase 9 UX:** `/memory` command surface in the TUI (list/show/
  forget/export), memory indicator, edit UX. The *API* exists behind the
  Backend boundary before then; the *UI* is Phase 9.
- **OpenCode side:** final injection boundary + version gate (2B);
  eventual plugin evaluation stays open but is not assumed.

---

## 20. Open questions for Phase 2B

1. **Storage/schema:** exact file layout (JSONL record shape, atomic
   append, tombstone format); user/project store locations (project store
   inside the repo dir — git-ignore policy?); whether V2 moves to
   SQLite+FTS5 and how records migrate; whether any Cargo dependency is
   ever justified (V1 uses std only).
2. **Retrieval algorithm:** ordering/budget math for injection
   (pinning + recency); FTS5 ranking once V2 lands; conflict surfacing UX.
3. **Injection boundary:** final choice among instruction entries /
   attachments / transform / synthetic message; version detection and gate
   mechanics; prefix-cache implications.
4. **Version resilience:** feature-flag schema, detection at connect
   (exists in Phase 4), degradation matrices.
5. **Security implementation design:** secret-pattern set, fence/label
   rendering, injection-as-data discipline, export/wipe formats.
6. **Final Adopt/Build/Hybrid decision:** clearly BUILD (minimal,
   adapter-side); formally ratify with the A/B/C re-score (§17) and record
   dissent.
7. **Event coverage (C3):** verify which adapter events can support a
   future capturer before V2 commits to rules extraction.

---

## ARCHITECTURE STATUS

```text
Candidate refined, implementation NOT authorized.
```

Phase 2A is research only. No memory engine, no store, no database, no
Cargo dependencies, no TUI/OpenCode modification. V1 (Phase 5) is not
authorized by this document; Phase 2B decides architecture, then Phase 5
implements.

---

## Final self-critique

1. **Did I blindly accept Phase 1's Option B?** No — B is demoted from
   "primary from the start" to the V2 growth path; A is recognized as the
   correct V1 (decision-log D1).
2. **Did I remove unnecessary concepts?** Yes — 6→3 components, 8→2
   kinds, 3→2 scopes, 5→3 states, 5 tables → 1 file store, all scores
   removed.
3. **Did I distinguish history from durable memory?** Yes (§3): history
   is what happened, memory is what we choose to re-inject; a durability
   test gates the boundary (§4).
4. **Did I justify every memory type?** Yes (§5): fact + preference kept,
   each with one reason; six others collapsed or deferred.
5. **Did I justify every scope?** Yes (§6): user (stable personal truth)
   and project (repo truth, isolated by placement); session is provenance.
6. **Did I avoid unnecessary scoring systems?** Yes (§8): zero stored
   scores; pinning, provenance, and lifecycle replace every number.
7. **Did I define contradiction/supersession clearly?** Yes (§10): five
   situations classified; silent overwrite forbidden; minimal lifecycle.
8. **Did I make provenance useful rather than enormous?** Yes (§11):
   four fields on the row answer "why does the system believe this?".
9. **Did I seriously question automatic extraction?** Yes (§14): V1 is
   explicit-only; rules and LLM return only behind opt-in gates.
10. **Did I seriously question Tier 1?** Yes (§15): it survives only as a
    name for "the whole corpus fits in context", not as a component.
11. **Did I preserve local-first behavior?** Yes — file store, local
    scopes, no services, no telemetry.
12. **Did I preserve OpenCode adapter isolation?** Yes — everything sits
    behind the existing `Backend` boundary (verified in `mod.rs`); the
    TUI is untouched.
13. **Did I preserve graceful failure?** Yes (§16): memory is optional,
    never on the turn's critical path, degrades to no-memory operation.
14. **Did I avoid a giant system for hypothetical futures?** Yes — future
    capabilities are *interfaces* (store trait, injection gate), not code.
15. **Could a developer understand the minimum architecture from the
    resulting documents?** Yes — `minimal-architecture.md` is
    intentionally small, with one diagram and a five-step data flow.