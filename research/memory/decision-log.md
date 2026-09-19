# Memory Research Phase 2A — Decision Log

> Every significant Phase 2A decision, with context, options, evidence,
> reasoning, rejected alternatives, confidence, and the condition under
> which it should be revisited. Research only; nothing implemented.
> Evidence references: `phase2a-memory-model.md` (P2A), `minimal-architecture.md`
> (MA), `comparison.md` (CM), `research-report.md` (RR), `architecture.md` (ARCH).

---

## D1 — Option A/B/C re-score: A is V1, B is the V2 growth path, C deferred

**Context:** Phase 1 recommended Option B (adapter-side engine) as primary
with A's Tier 1 and C's isolation (ARCH §36).

**Options:**
1. Keep B primary from the start (Phase 1 position).
2. A for V1 (explicit curated, in-context), B as the V2 growth path behind
   the same interfaces, C deferred.
3. C (sidecar) from the start.

**Evidence:** V1 execution is user-curated and bounded P2A §14; a retriever
and extractor are unjustified until the corpus overflows the injection
budget P2A §15/§17; a sidecar adds a process and interface complexity with
no V1 consumer P2A §17, MA §7.5.

**Decision:** A is the correct V1 shape; build the `MemoryStore` interface
and command surface so B (SQLite/FTS5 + extraction + retrieval) slots in
without rework; C remains future (embeddings/graph).

**Reason:** The smallest useful first implementation is a curated store
whose entire corpus fits the context budget. Building B's machinery first
would be solving a scale problem that has not arrived.

**Rejected alternatives:** B-first (premature infrastructure); C-first
(highest complexity, no justified consumer).

**Confidence:** High.

**Revisit condition:** If a V1 deployment shows the corpus routinely
overflowing the injection budget, pull B's retrieval forward into V1.

---

## D2 — SESSION is not a memory scope

**Context:** Phase 1 proposed USER / PROJECT / SESSION scopes (ARCH §35.3).

**Options:**
1. First-class SESSION scope.
2. Session as temporary working memory (pinned session notes).
3. Session represented through OpenCode history only.
4. Session as provenance only; excluded from V1 scopes.

**Evidence:** OpenCode already stores session history (session/message
APIs, local SQLite — RR §38.1, **VERIFIED**); "durable memory" is defined
as information carried *beyond* the originating session P2A §3; a
session-scoped store would duplicate history without a retrieval
consumer P2A §6.

**Decision:** SESSION is provenance (`session_id`, `source_ref`, quote),
not a memory scope. Scopes are exactly {user, project}. Pinned session
notes (option 2) are a possible later addition, not V1.

**Reason:** "What happened this session" is history's job; "what this
session established durably" is a project/user fact with session provenance.

**Rejected alternatives:** 1 (creates history duplication), 2 (unneeded in
V1; no consumer).

**Confidence:** High.

**Revisit condition:** If a strong V1 use case for ephemeral cross-turn
working notes appears, revisit as a separate "notes" feature — not as a
memory scope.

---

## D3 — V1 stores no scores: no trust, confidence, importance, novelty, proof count, retrieval count

**Context:** Phase 1 floated trust, confidence, importance, novelty,
durability, evidence count, retrieval count (ARCH §35.4; CM §12).

**Options:**
1. Store full scoring set.
2. Store a subset (e.g., trust only).
3. Store none; use provenance + lifecycle + user pinning.

**Evidence:** Holographic's trust requires feedback math that only matters
once machine extraction writes rows (CM §15, **VERIFIED** in
`holographic.md`); no V1 mechanism consumes any score P2A §8; importance is
user opinion better expressed by pinning/order P2A §8:3; scores nobody can
explain are worse than no scores P2A §8:1.

**Decision:** Zero stored score fields in V1. Explaining power comes from
provenance; ordering comes from pin-then-recency; change comes from
lifecycle.

**Reason:** Every candidate score lacked a well-defined action in V1; each
had a cheaper, explainable replacement.

**Rejected alternatives:** 1 (unexplainable numbers, no consumers), 2
(trust without extraction is unreachable).

**Confidence:** High.

**Revisit condition:** When rules/LLM extraction (V2+) writes `inferred`
rows, revisit an optional feedback-based trust under the `MemoryStore`
trait — never as a V1 hard dependency.

---

## D4 — V1 ingestion is explicit user command only (Model A)

**Context:** Phase 1 assumed rules-first + opt-in LLM extraction in the
engine (ARCH §35.2; CM §13).

**Options:**
1. Model A — explicit user-controlled memory only.
2. Model B — rules-based automatic extraction.
3. Model C — LLM-assisted extraction.
4. Model D — hybrid (A+B+C).

**Evidence:** Rules extraction produces noise that corrupts context
(P2A §14.2:1); LLM extraction needs a configured model the
backend-agnostic adapter cannot assume (P2A §14.2:3); event-driven capture
depends on unverified Claim C3 (ARCH §34.4, **[INFERRED]**); explicit
capture is costless, private, and perfectly vetted (P2A §14.1 row A).

**Decision:** V1 = Model A exclusively. V2 = narrow rules extraction with
an ASK gate (after C3 verification). V3 = opt-in LLM extraction. The
record shape and store queries are identical for all producers.

**Reason:** Wrong memory is worse than no memory; the user is the only
authority that vets V1 rows, and the only producer.

**Rejected alternatives:** B (noise, unverified coverage), C (model
dependency, unauthorized in the adapter), D (compound complexity).

**Confidence:** High.

**Revisit condition:** If a V1 user study shows memory adoption collapses
because manual capture is too onerous, introduce narrow B-type rules behind
an ASK gate in V2.

---

## D5 — Minimal lifecycle: ACTIVE / SUPERSEDED / DELETED; CONFLICT reserved; ARCHIVED dropped; silent overwrite never

**Context:** Phase 1 candidate statuses were active/superseded/
flagged_conflict (+ borrowings: archived, deleted) (ARCH §35.3).

**Options:**
1. Five-state model (as Phase 1 listed).
2. Three states now + CONFLICT reserved, ARCHIVED dropped.
3. Two states (ACTIVE + hidden) relying on deletion alone.

**Evidence:** Silent overwrite is uniformly rejected by surveyed systems
(Mem0 ADD-only, Holographic surfaced conflicts, Hindsight
refine-with-history, Hermes failure cap — CM §15, **VERIFIED**); CONFLICT
is unreachable until machine extraction exists P2A §10.3; ARCHIVED has no
V1 consumer (forget covers it) P2A §10.3; supersession is needed for
explainable correction P2A §13.3.

**Decision:** States = ACTIVE / SUPERSEDED / DELETED in V1; CONFLICT
defined in the model but implemented in V2; ARCHIVED removed; silent
overwrite forbidden by design.

**Reason:** Identified-state coverage at minimum cost; correction is a
visible supersession, never a replacement.

**Rejected alternatives:** 1 (states without behavior), 3 (loses
explainable history of change).

**Confidence:** High.

**Revisit condition:** If a "park indefinitely but stop injecting" use case
arises, reintroduce ARCHIVED — cheap, additive.

---

## D6 — Fact/inference plus confidence collapse into one `source ∈ {user, inferred}` label

**Context:** Phase 1 proposed binary fact/inference labeling per row and
treated confidence as a separate concept (ARCH §35.3; P2A §7).

**Options:**
1. `fact_inference` column + confidence score.
2. Single `source` label; no confidence.
3. No label (rely on provenance text alone).

**Evidence:** The only axis that must survive is *who vouches*
(quote = evidence, label = status — P2A §7.1); confidence is a number
nobody updates or acts on in a curated corpus (P2A §7.2:2); in V1 every
row is user-stated so the invariant is free (P2A §7.1).

**Decision:** One `source` field: `user` (vouched) or `inferred`
(machine-derived, V2+). No confidence field. The label travels with the row
and is rendered in the injected block.

**Reason:** Confidence's job (help the reader judge) is done by the quote +
label; a stored confidence number adds no action.

**Rejected alternatives:** 1 (redundant scoring), 3 (loses the invariant
before extraction exists).

**Confidence:** High.

**Revisit condition:** If V3 LLM extraction needs a decision threshold
internally, compute a transient score at extraction time — never persist it
per row.

---

## D7 — Provenance is four fields on the record; no provenance/events/entities tables

**Context:** Phase 1 candidate schema had `provenance`, `events`, and
`entities` tables alongside `memories` (ARCH §35.3).

**Options:**
1. Separate tables: memories, entities, provenance, events (+ FTS5).
2. Provenance as 4 columns on the row; no tables; entities deferred.

**Evidence:** Provenance is 1:1 with a memory in V1 (single source:
the user's own command + session) so a table adds joins without value
(P2A §11.2); no content-bearing audit is wanted locally (P2A §13.2:3);
entity text is FTS-searchable without an entity layer (P2A §5:3).

**Decision:** Record carries `session_id`, `source_ref`, `quote ≤ 256`,
`created_at`. No provenance table, no events table, no entity table.

**Reason:** Simplicity without losing any answer to "why does the system
believe this?"; audit/deletion policy is handled by tombstones (§D8).

**Rejected alternatives:** 1 (relational overhead for 1:1 data; audit of
deleted content conflicts with privacy stance).

**Confidence:** High.

**Revisit condition:** If V2 extraction produces many-to-one evidence
(multiple sources per memory), introduce an evidence list — behind the
store implementation, not the API shape.

---

## D8 — V1 storage is a plain file store (JSONL); SQLite+FTS5 is re-evaluated in 2B/V2; no new Cargo deps

**Context:** Phase 1 recommended SQLite + FTS5 for Tier 2 and implied a
Cargo dependency (ARCH §34.4 C7; CM §27).

**Options:**
1. SQLite + FTS5 in V1.
2. File store (JSONL) in V1 behind a `MemoryStore` trait; SQLite in V2.
3. In-memory only (no persistence).

**Evidence:** Corpus in V1 is user-curated and budget-bounded, so the
query surface is "read ACTIVE rows, ordered, budgeted" — no full-text
search needed yet P2A §15; no-dependency constraint from project rules
(AGENTS.md) and Phase 1 (CM §26); the store trait makes a later swap
transparent P2A §17, MA §7.1.

**Decision:** Plain append-only JSONL files (project + user stores) with
atomic-append writes and a hash tombstone set. No new Cargo dependencies.
Phase 2B re-evaluates SQLite+FTS5 for V2 when extraction + corpus size
justify search.

**Reason:** Smallest durable persistence that still supports scope
isolation, tombstones, ordering, and manual backup/export; avoids coupling
V1 to a database schema.

**Rejected alternatives:** 1 (premature; freezes schema/deps before the
corpus exists), 3 (durability is the point of memory).

**Confidence:** High for V1; the *choice of* SQLite in V2 is a 2B decision.

**Revisit condition:** If V1 shows reliability/writes problems (concurrent
access, corruption), move to SQLite earlier — the interface is already
defined.

---

## D9 — Entities are deferred to V3; no entity handling in V1

**Context:** Phase 1 candidate had entity extraction + an `entities` table.

**Options:**
1. Entity table + extraction in V1.
2. Plain-text search only in V1; entity graph in V3.
3. Entity tags as optional free-form metadata now.

**Evidence:** "PostgreSQL" in "project uses PostgreSQL" is recovered by
text search P2A §5:3; entity relationships are graph machinery (memory
`relationship` type) with no V1 consumer P2A §5; tags drive no behavior in
V1 P2A §5 verdict.

**Decision:** No entity extraction, no entity table, no tags in V1. Text
search (and the future SQLite impl) covers entity-name recall; the entity
graph is V3/Phase-8 sidecar work.

**Reason:** Entities are a retrieval optimization; there is no retrieval in
V1. Building them now is speculative infrastructure.

**Rejected alternatives:** 1 (speculative), 3 (tags still need a consumer
to be worthwhile).

**Confidence:** High.

**Revisit condition:** When semantic retrieval (embeddings/graph, V3)
becomes a goal, derive entities from stored text at that point.

---

## D10 — Temporal model is created_at + updated_at; no temporal reasoning in V1

**Context:** Phase 1 listed occurred_at / learned_at / updated_at /
superseded_at / expires_at (ARCH §35.3; P2A §9).

**Options:**
1. Full six-column temporal model.
2. created_at + mechanical updated_at; supersession encodes change.
3. No timestamps (order only).

**Evidence:** "Occurred" vs "learned" only differ once an extractor
observes events independently of storing them P2A §9.1; current-vs-
historical facts are different propositions coexisting, and same-proposition
change is supersession — neither needs temporal reasoning P2A §9.2;
expires_at is session-context territory P2A §9.1.

**Decision:** `created_at` (ordering, display) + mechanical `updated_at`
(maintenance). `superseded_at` is derivable from status + updated_at.
No `occurred_at`/`learned_at`/`expires_at` in V1. No temporal reasoning in
V1.

**Reason:** Provenance + lifecycle + user-authored text answers every
temporal question V1 must handle.

**Rejected alternatives:** 1 (six timestamps, four without consumers), 3
(cannot show when something was learned).

**Confidence:** High.

**Revisit condition:** V3 temporal analysis (e.g., "what changed since
March?") would add occurred_at — additive, behind the store trait.

---

## D11 — "Tier 1" is not a separate layer; it is the V1 store's injection mode

**Context:** Phase 1 defined Tier 1 (always-in-context curated files) as a
distinct design layer from Tier 2 (SQLite engine) (ARCH §37).

**Options:**
1. Two layers in V1 (file Tier 1 + SQLite Tier 2).
2. One store; whole-corpus budgeted injection in V1; retrieval replaces it
   in V2 when the corpus outgrows the budget.
3. Tier 1 only, no Tier 2 plan.

**Evidence:** At V1 scale the whole corpus fits the injection budget, so
"always in context" and "the store's default read" are the same operation
P2A §15.3; a separate Tier-1 mechanism would duplicate the budgeted-read
path P2A §17.

**Decision:** One store. Injection = ACTIVE rows, pinned-then-recency,
char-budgeted, frozen per session. When corpus growth + extraction (V2)
require ranking, retrieval becomes the implementation behind the same
Context Builder — no separate "Tier 1" component.

**Reason:** Remove the Phase 1 vertical split; keep the useful property
(small, stable, always-in-context user + project blocks) as a mode, not a
module.

**Rejected alternatives:** 1 (duplicated mechanism), 3 (would cap growth
without a path).

**Confidence:** High.

**Revisit condition:** If a V1 deployment shows the always-in-context block
hurting token budgets (very large project/user corpora), introduce
per-turn retrieval in V2 rather than a Tier-1/Tier-2 split.

---

## D12 — V1 injection is session-start, whole-corpus-when-in-budget, frozen; no per-turn retrieval

**Context:** Phase 1 left injection strategy open (session-start vs
per-turn).

**Options:**
1. Session-start injection; frozen per session (prefix-cache).
2. Per-turn retrieval.
3. Retrieval only when relevant (on-demand).

**Evidence:** V1 corpus fits the budget so per-turn retrieval has nothing
to add P2A §15.2; frozen blocks preserve prefix-cache stability
(Hermes snapshot pattern, **VERIFIED** in `hermes.md`) P2A §15.2; stale
risk is bounded by small size + refresh at session start + user visibility
P2A §15.2.

**Decision:** Session-start, frozen block; refresh at each session start;
stale-risk accepted mid-session; per-turn retrieval is V2+.

**Reason:** Cheapest correct behavior at the current scale; cache-friendly;
fully explainable.

**Rejected alternatives:** 2/3 (need a retriever and per-turn cost that V1
does not have).

**Confidence:** High.

**Revisit condition:** When corpus overflow or mid-session staleness
becomes observable (V2), switch to per-turn retrieval behind the same
Context Builder entry point.

---

## D13 — Retention: creation requires explicit user request in V1; ASK arrives with V2 extraction; secrets are refused, not redacted

**Context:** Phase 1 had no explicit retention policy step (implicit in
extraction design, ARCH §35.2).

**Options:**
1. AUTO-STORE for everything detected.
2. Explicit-request-only (V1); ASK gate for V2 rule candidates.
3. Never store automatically, ever.

**Evidence:** Auto-store without a trusted extractor = noise (P2A §14.2 and
D4); secret categories must be NEVER-STORE and refusal is clearer than
redaction in a local, user-owned store P2A §12.3; repeated behavior and
explicit corrections are the V2/V1 gates respectively P2A §12.1.

**Decision:** V1 = AUTO-STORE on explicit command; NEVER-STORE for secrets
(refused, pattern-based) and transient/debug material; ASK gate introduced
with V2 rules extraction; LLM-extracted rows labeled `inferred` and never
presented as user-stated.

**Reason:** User-request-only keeps quality at 100% while the user is the
sole producer; the ASK gate is the bridge to automation.

**Rejected alternatives:** 1 (depends on unbuilt extraction), 3 (no growth
path).

**Confidence:** High.

**Revisit condition:** If V2 extraction quality studies show low
false-positive rates, allow some categories to pass without ASK —
preferences/project facts first.

---

## D14 — Deletion = hard delete + hash tombstone; correction = supersession; no content audit trail

**Context:** Phase 1 did not specify a deletion model (soft/hard/tombstone).

**Options:**
1. Hard delete only.
2. Hard delete + tombstone hash.
3. Soft delete (status=DELETED retained).
4. Soft delete + audit log.

**Evidence:** Tombstones prevent re-adoption by future extraction/import —
the resurrection risk is real once V2 dedup exists P2A §13.1:2; content
audit trails conflict with the local privacy stance P2A §13.2:3; correction
is supersession, not deletion P2A §13.3.

**Decision:** `forget` = hard delete of the row + provenance, plus a
tombstone `{content_hash, deleted_at}` with no content. Correction =
same-key `remember` writing a new ACTIVE row and marking the old
SUPERSEDED. No audit of deleted content in V1.

**Reason:** Trustworthy absolute deletion (privacy) without resurrection
(integrity through the tombstone), and no way for a deleted fact to be
re-injected silently.

**Rejected alternatives:** 1 (resurrection risk in V2+), 3/4 (retains
content the user asked to erase).

**Confidence:** High.

**Revisit condition:** If V2 introduces derived/inferred rows, deletion
bubbling (delete rows whose only basis was deleted) becomes required — see
P2A §13.2:5.

---

# Phase 2B decisions (2026-09-18)

> Phase 2B final decisions. Each reverses, refines, or ratifies a Phase 2A
> decision; reversals are explicit, never silent edits of history. Evidence
> refs: `phase2b-storage.md` (ST), `phase2b-retrieval.md` (RT),
> `phase2b-injection.md` (INJ), `phase2b-security.md` (SEC),
> `phase2b-version-resilience.md` (VR), `phase2b-decision-report.md` (DR).

## D15 — Record store is rewrite-on-mutation JSONL; tombstones are a separate append-only file — [REFINES D8]

**Context:** D8 said "plain append-only JSONL". Strictly append-only
record files cannot hard-delete (the content stays on disk), contradicting
D14's privacy semantics.

**Options:** A. mutable in-place snapshot · B. append-only event log ·
C. append + periodic compaction · **D. rewrite-on-mutation + atomic
rename** · E. separate append-only tombstone file.

**Evidence:** Corpus is curated and budget-bounded (P2A §15), so full
rewrites are milliseconds (ST §5); rename-swap gives the strongest
crash-safety without dependencies (ST §6); hard deletion is *real*
(content absent after rename); B/C need a compaction subsystem V1 does not
use (ST §5). Tombstone content must not remain inside the file the user
emptied (ST §11.3).

**Decision:** The memory record file is rewritten in full per mutation via
temp + `fsync` + atomic `rename`; tombstones live in a separate
**append-only** `tombstones.jsonl` (`O_APPEND` + `fsync`). The
`MemoryStore` trait contract is unchanged (P2A §17).

**Reason:** File-is-state makes reads lock-free, corruption trivial to
recover from, exports/git-diffs trivial, and the future SQLite migration a
state serialization (ST §14–§15).

**Rejected alternatives:** B/C (unneeded compaction machinery; deleted
content lingers), A (random-access torn-write risk).

**Confidence:** High.

**Revisit condition:** If V1 shows write-frequency problems at real scale,
push compaction/SQLite earlier — trait already isolates it.

**Status:** Phase 2B final.

---

## D16 — Exact V1 record schema `v:1` — [RESOLVES P2A inconsistencies]

**Context:** Phase 2A left the field set open (P2A §7.3 vs §15 vs §5
verdict), with three internal gaps: pinning referenced by injection design
but absent from the record; `tags` contradiction between P2A §5 ("stay in
the record shape") and D9 ("no tags in V1"); `quote`/`source_ref`
"required" though V1 adapter cannot always identify them.

**Options:** Keep the loose candidate set vs fix it precisely.

**Evidence:** Phase 4 verified the adapter knows only the session id at
command time (ST §4, **VERIFIED**); D9 is the authoritative no-tags
decision (ST §4.1); `pinned` must exist as a field for the D12 injection
order to be implementable.

**Decision:** Record = `v:1, id, key?(^[a-z0-9][a-z0-9._-]{0,63}$,
ACTIVE-unique per scope, lowercase), kind(fact|preference),
scope(user|project), content(≤4096), source(user; inferred reserved),
status(ACTIVE|SUPERSEDED; DELETED reserved), pinned(bool),
created_at/updated_at(RFC3339 UTC), session_id(required),
source_ref?(optional), quote?(≤256, optional)`. No tags, no scores.

**Reason:** Every field has a documented consumer; the three gaps are
closed explicitly (ST §4–§4.2).

**Rejected alternatives:** keeping tags (D9), keeping quote/source_ref
required (V1 adapter can't fill them), dropping pinned (breaks D12).

**Confidence:** High.

**Revisit condition:** V2 extraction adding production metadata (method/
model) extends the schema under `v` — additive, versioned.

**Status:** Phase 2B final.

---

## D17 — Injection mechanism: instruction entries, single `owt.memory` entry — [RATIFIES P2A §15.4 with fresh verification]

**Context:** P2A left the boundary choice to 2B (P2A §20/Q3).

**Options:** 1. instruction entries · 2. plugin system-prompt transform ·
3. synthetic message · 4. prompt attachments · 5. AGENTS.md files.

**Evidence:** Instruction entries **live-verified on the running 2.0.8
server** (PUT/GET/DELETE 204; 413 `InstructionEntryValueTooLargeError`
maxBytes=262144; non-experimental alias 404s on 2.0.8) (INJ §1,
**VERIFIED**); documented at prompt-assembly position 6 (INJ §1.1,
**DOCUMENTED**); V2 has **no** plugin system-prompt hook and plugin-defined
Context Sources are deferred (INJ §1, **DOCUMENTED**); synthetic messages
pollute the user-visible transcript (INJ §10); AGENTS.md writes violate
adapter isolation (INJ §10).

**Decision:** V1 injection = a **single** instruction entry `owt.memory`
holding the whole fenced block as `{"value":{"text":"<block>"}}`, written
at session creation before the first prompt. Entries are the fallback-free
primary; synthetic message documented as the degraded path only; never
AGENTS.md/plugins/attachments.

**Reason:** The only adapter-addressable surface that lands in the
system-context assembly (position 6) without polluting history; key
pattern fits our `owt.memory` key; the 262,144 B limit becomes the hard
ceiling for our budget invariant (RT §3).

**Rejected alternatives:** 2 (no V2 hook; in-process only), 3 (history
pollution), 4/5 (isolation, repo mutation).

**Confidence:** High (mechanism verified); value-shape rendering is
**[UNRESOLVED]** → Phase 6 sandbox probe (DR §8).

**Revisit condition:** If OpenCode removes the entries surface, V1 keeps
store commands and degrades injection off (VR §3); the fallback chain does
not add synthetic in V1.

**Status:** Phase 2B final.

---

## D18 — Session-start frozen injection is mandatory in V1 — [RATIFIES D12]

**Context:** D12 chose session-start frozen on cache-stability grounds.

**Options:** frozen-mandatory · configurable · per-turn retrieval.

**Evidence:** The documented V2 Context Epoch model makes session-start
placement *native*: an immutable provider-cache baseline is initialized
before the first prompt, reused verbatim across restarts, and the session
id is the prompt-cache key; mid-session changes are admitted only as
chronological System messages at the next safe provider-turn boundary
(INJ §1.2, **DOCUMENTED**). Per-turn retrieval would fight the runtime's
own caching model.

**Decision:** Frozen at session start, **mandatory** in V1; the block PUT
must complete before the first prompt (adapter sequencing requirement);
no mid-session entry writes; per-turn retrieval is V2+, replacing the
Context Builder mechanism wholesale behind the same interface.

**Reason:** Cache-optimal, deterministic, cheap, single-failure-surface
(INJ §7).

**Rejected alternatives:** configurable-frozen (no V1 consumer for the
option), per-turn (breaks baseline stability; D12 rationale reinforced by
fresh documentation).

**Confidence:** High.

**Revisit condition:** Observable mid-session staleness at scale (V2).

**Status:** Phase 2B final.

---

## D19 — Memory command surface = adapter-side `/memory` prefix routing; TUI unchanged until Phase 9 — [RESOLVES the 2A contradiction]

**Context:** P2A proposes `remember/update/forget/list/show` while saying
"TUI unchanged until Phase 9" — an apparent contradiction.

**Options:** 1. TUI command UI in V1 · 2. adapter-side prefix interception
of normal message text · 3. OpenCode config slash-commands
(rejected: modifies OpenCode integration).

**Evidence:** The adapter owns message submit + session id (Phase 4
**VERIFIED**); interception needs zero TUI change; OpenCode-side command
config would violate adapter isolation (INJ §8).

**Decision:** The adapter intercepts messages beginning `/memory ` (alias
`/mem `), parses, routes to the Memory API, replies in-band; everything
else forwards unchanged. Escape hatch for literal text documented. The
Phase 9 TUI delivers the *UI*; the *API* exists behind the Backend
boundary from Phase 5.

**Reason:** Resolves the contradiction without moving memory into the TUI
or into OpenCode.

**Rejected alternatives:** 1 (Phase 9 scope), 3 (isolation violation).

**Confidence:** High.

**Revisit condition:** OpenCode adopting a conflicting root command named
`/memory`; then namespace further.

**Status:** Phase 2B final.

---

## D20 — Project store at `<project-root>/.owt/`, git-ignored and private by default

**Context:** P2A set project-local placement but left the exact location
and git policy to 2B (P2A §20/Q1).

**Options:** locations: `.owt/` vs `.opencode/…` vs `.memory/`; git:
committed vs ignored.

**Evidence:** `.opencode/` is OpenCode's own directory (never write into
it — adapter isolation); project memory overlaps private developer
preferences and is not guaranteed secret-free (SEC §4), so committing it
without a review gate is a privacy failure; JSONL export is trivial when
portability is wanted (ST §10).

**Decision:** `<project-root>/.owt/{memory,tombstones}.jsonl` + lock;
`.owt/` is **git-ignored by default**; committing is an explicit
opt-out; portability = copying `.owt/` (export CLI in Phase 5, import in
V2, tombstone-guarded). Project identity is the canonical path
(worktrees/renames = separate stores, documented limitation).

**Reason:** Private-by-default until there is a reviewed, reviewed-export
story; isolation from OpenCode's directory; path identity is what the
adapter already knows.

**Rejected alternatives:** `.opencode/…` (isolation), `.memory/`
(collision-prone), committed-by-default (privacy).

**Confidence:** High.

**Revisit condition:** A desire for team-shared project memory; then
design a review surface + import/export before flipping the default.

**Status:** Phase 2B final.

---

## D21 — Concurrency & crash-safety model: flock + in-process mutex + atomic rename; readers lock-free

**Context:** P2A left atomicity/concurrency unspecified (P2A §20/Q1).

**Options:** 1. no locking (last-writer-wins) · **2. flock + mutex +
read-under-lock + rename** · 3. SQLite to get locking for free.

**Evidence:** flock is per-fd — threads sharing an fd need a mutex too
(ST §7); rename-swap makes reader access torn-free with no reader lock
(ST §6/§7); D8 keeps V1 dependency-free; malformed-line degrade is the
recovery contract (ST §8).

**Decision:** Mutations: process mutex → `flock(LOCK_EX)` → re-read under
lock → write temp → fsync → rename → dir-fsync. Readers: no lock. Stale
temp cleaned on store open. Torn tombstone tail ignored. Corrupt lines
skipped+warned, never fatal, never auto-repaired on read. 10 MiB load
guard.

**Reason:** Strongest dependency-free crash/concurrency safety for a
one-writer local store.

**Rejected alternatives:** 1 (lost updates), 3 (premature; D8/ST §14).

**Confidence:** High.

**Revisit condition:** Observed contention or corruption at scale → SQLite
swap behind the trait (D8 revisit).

**Status:** Phase 2B final.

---

## D22 — Final decision: BUILD — [RATIFIES D1; ADOPT/HYBRID rejected for V1]

**Context:** Phase 2B brief §27 requires a final Adopt/Build/Hybrid
decision against concrete V1 requirements.

**Options:** ADOPT (Hermes/Hindsight/Mem0/Letta/OpenViking) · BUILD ·
HYBRID.

**Evidence:** No surveyed framework solves even two of the nine V1
requirements (R1 explicit-only, R2 no scoring, R3 no ranking, R4 no deps,
R5 adapter-side isolation, R6 local files, R7 version resilience, R8
privacy, R9 determinism); all assume extraction/embedding/ranking or a
runtime, which are V1 anti-features (DR §3). Hybrid elements (FTS5, trust,
embeddings) are V2/V3 growth behind the `MemoryStore` trait (D3).

**Decision:** **BUILD** the three-component V1 (DR §2). None of the
frameworks is embedded; each is documented as rejected with its rationale
(Hermes/Hindsight/Mem0/Letta/OpenViking, DR §3).

**Reason:** The requirements are anti-features of every framework; value is
in the minimal contract, not in adopted machinery.

**Rejected alternatives:** ADOPT (violates R1/R2/R4/R5 by construction),
HYBRID-in-V1 (premature; same violations subset).

**Confidence:** High; dissent recorded (DR §3).

**Revisit condition:** Only if V1 requirements change — not as a function
of framework novelty.

**Status:** Phase 2B final.

---

## D23 — Secret policy: high-confidence refusal, warn-on-label, honest limits

**Context:** P2A §12.3 proposed refusal; the 2B brief §23 asks for exact
patterns and honest limits.

**Options:** 1. giant regex zoo + hard refusal everywhere · **2. small
high-confidence set refused; label/value proximity warned; label-only
stored** · 3. redaction.

**Evidence:** Providers are out of trust scope (injected memory *is* sent
to them — SEC §9), so secrets should not be stored; label-only matches
have high false-positive rates ("my password manager setup"); "secret-free"
is unachievable without ML (SEC §4.3).

**Decision:** Refuse (hard error, no state change) on: provider API-key
shapes (`sk-…`/`pk-…`), PEM private-key armor, `ghp_…`/`github_pat_…`,
`AKIA…`, `xox…`. Warn-but-store on label+value proximity (e.g.
`password:` with a value, `user:pass@` URLs). Label-only text stores
silently. No giant regex zoo; deterministic std-regex list; documented
limits; never claimed as a security boundary (real boundary = 0600 +
machine, SEC §2/§4).

**Reason:** Ergonomics without false-blocking legitimate memory; honest
about non-guarantees.

**Rejected alternatives:** 1 (false positives block real memory), 3
(mangles data; P2A §12.3).

**Confidence:** High.

**Revisit condition:** Escalate warning rows to refusal if injected
warning-flagged content is ever observed in the wild (SEC §4.2).

**Status:** Phase 2B final.

---

## D24 — Retrieval: deterministic tier ordering + 12k-char budget; no search in V1

**Context:** Phase 2B brief §11–§12: exact ordering and whether V1 needs
any search.

**Options:** orderings (pinned-then-recency vs recency-only vs scope-
weighted); budget units (chars/tokens/bytes); with/without substring·FTS·
vector search.

**Evidence:** The hard ceiling is the measured 262,144-byte entry limit
(ST §2, **VERIFIED**); corpus is budget-bounded so ranking has nothing to
rank against (RT §1/§7); forget-by-query is a footgun — exact key/id only
(RT §7.1); determinism preserves the V2 provider-cache prefix (RT §9,
**DOCUMENTED**).

**Decision:** Ordering = tiers project-pinned → user-pinned →
project-recent → user-recent; within tier `updated_at DESC, created_at
DESC, id ASC` (total order). Budget = 12,000 chars default (configurable,
≤30,000) with hard invariant: JSON-encoded block ≤200,000 B under the
server's 262,144 B; whole-record truncation; encode-bounded truncation
loop for non-ASCII. No substring/token/fuzzy/FTS5/vector/semantic search
in V1.

**Reason:** Specificity-first budget survival; byte-identical snapshots;
search has no V1 consumer.

**Rejected alternatives:** recency-only (drops pinning), token budgets
(less readable), any search (none consumed; RT §7).

**Confidence:** High.

**Revisit condition:** Corpus overflow or observable staleness (V2) →
per-turn retrieval behind the same read shape.

**Status:** Phase 2B final.

---

## D25 — Ingestion and load limits

**Context:** 2B brief §22 DoS/oversized concerns.

**Options:** unbounded vs capped.

**Evidence:** Control characters and NULs are hostile to rendering (SEC
§6); oversized single records poison the budget loop (RT §3.1); huge
stores are a local DoS vector (SEC §7).

**Decision:** Per-record caps: content ≤4,096 chars, quote ≤256, key ≤64;
NUL + C0 controls rejected at ingestion; store files >10 MiB refused to
load (memory unavailable + warning); builder single-record-too-big
drop-with-warning; no lossy decoding (SEC §6/§7).

**Reason:** Cheap static caps bound every abuse path without machinery.

**Rejected alternatives:** unbounded (abuse), streaming chunking (V1
doesn't need it).

**Confidence:** High.

**Revisit condition:** Real memory content hitting the 4,096-char cap
frequently → raise with a documented reason, not silently.

**Status:** Phase 2B final.

---

## Decision summary table (Phase 2A + Phase 2B)

| # | Decision | Confidence | Part of V1? |
|---|---|---|---|
| D1 | A=V1, B=V2 growth path, C deferred | High | yes |
| D2 | No SESSION scope; provenance only | High | yes |
| D3 | Zero stored scores | High | yes |
| D4 | Explicit-user-only ingestion | High | yes |
| D5 | ACTIVE/SUPERSEDED/DELETED; no overwrite | High | yes |
| D6 | Single `source` label; no confidence | High | yes |
| D7 | Provenance on the row; no aux tables | High | yes |
| D8 | File store V1; SQLite re-evaluated 2B | High | yes |
| D9 | No entities until V3 | High | yes |
| D10 | created_at + updated_at only | High | yes |
| D11 | No separate Tier 1 | High | yes |
| D12 | Session-start frozen injection | High | yes |
| D13 | Explicit-request retention; secrets refused | High | yes |
| D14 | Hard delete + tombstone; supersession corrects | High | yes |
| D15 | Rewrite-on-mutation record store + append-only tombstones (refines D8) | High | yes |
| D16 | Exact `v:1` schema; no tags; pinned; optional quote/source_ref (fixes P2A §5/§7.3 vs D9) | High | yes |
| D17 | Injection = instruction entries; single `owt.memory`; 262,144 B measured limit | High (rendering UNRESOLVED → Phase 6) | yes |
| D18 | Session-start frozen mandatory in V1 | High | yes |
| D19 | `/memory` prefix routed adapter-side; no TUI change | High | yes |
| D20 | `<root>/.owt/` git-ignored, private by default | High | yes |
| D21 | flock + mutex + atomic rename; lock-free readers; degrade | High | yes |
| D22 | **BUILD**; ADOPT/HYBRID rejected for V1 | High | yes |
| D23 | Secret refusal (small set) + warn-on-label; honest limits | High | yes |
| D24 | Deterministic tier ordering; 12k-char/≤200,000 B budget; no search | High | yes |
| D25 | Ingestion/load caps (4,096 chars; 10 MiB guard; control-char rejection) | High | yes |