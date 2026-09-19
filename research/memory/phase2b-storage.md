# Memory Research Phase 2B — Storage & Schema Design

> Status: 🔬 RESEARCH ONLY — Phase 2B design output. **Nothing here is
> implemented.** This document resolves the open storage questions carried
> from Phase 2A (`phase2a-memory-model.md` §20/Q1, decision-log D8):
> the physical JSONL representation, the append-only vs. mutation
> contradiction, atomicity/crash safety, concurrency, store locations,
> git interaction, tombstone semantics, key semantics, and the JSONL-vs-
> SQLite re-evaluation.
>
> Evidence labels used throughout: **[VERIFIED]** live probe of the
> running OpenCode 2.0.8 server · **[DOCUMENTED]** official OpenCode
> docs/specs · **[OBSERVED]** seen but not confirmed · **[INFERRED]**
> analysis · **[RECOMMENDATION]** Phase 2B decision → Phase 5 review.
>
> Companion documents: `phase2b-retrieval.md`, `phase2b-injection.md`,
> `phase2b-security.md`, `phase2b-version-resilience.md`,
> `phase2b-decision-report.md`, `decision-log.md` (D15–D24).

---

## 1. Final storage decision (summary)

| Question | Decision |
|---|---|
| Format | JSONL, one complete record per line, in a **rewrite-on-mutation** file (Option D, §5) |
| Mutations | Full-file rewrite to a temp file + `fsync` + atomic `rename` (§6) |
| Tombstones | Separate **append-only** `tombstones.jsonl` (`O_APPEND` + `fsync`) (§8) |
| Concurrency | In-process mutex + cross-process `flock` on a lock file; readers lock-free (§7) |
| Corruption | Malformed lines skipped + counted + warned; never fatal, never auto-rewritten at read time (§8) |
| User store | `$XDG_DATA_HOME/owt/` (Linux default `~/.local/share/owt/`) (§9) |
| Project store | `<project-root>/.owt/`, **git-ignored by default** (§10) |
| Schema | Exact record defined in §4 (D16) |
| SQLite+FTS5 | Deferred to V2; re-evaluated in §14 |

---

## 2. Evidence base for this phase

All OpenCode behavior claims below were re-verified during Phase 2B, not
carried over from Phase 1:

| Claim | Label | Source |
|---|---|---|
| Instruction-entry API exists on the running server: `GET/PUT/DELETE /api/experimental/session/{id}/instructions/entries[/{key}]`; PUT/DELETE → 204 | **[VERIFIED]** | Live probe, OpenCode 2.0.8 (this phase) |
| Entry value limit: 413 `InstructionEntryValueTooLargeError`, `maxBytes = 262144` (256 KiB) | **[VERIFIED]** | Live probe with 1 MiB / 10 MiB values (this phase) |
| Non-experimental alias `/api/session/{id}/instructions/entries` → 404 on 2.0.8 despite being documented | **[VERIFIED]** | Live probe (this phase) |
| Entry key pattern `^[a-z0-9][a-z0-9._-]*$` | **[VERIFIED]** | OpenAPI spec `InstructionEntry.Key` |
| PUT request body = `{"value": <any JSON>}` (additionalProperties false) | **[VERIFIED]** | OpenAPI spec (this phase) |
| Entries compose into the prompt at assembly position 6 ("Session-specific instruction entries supplied through the API") | **[DOCUMENTED]** | opencode.ai/v2/docs/instructions |
| "Attach or replace one durable instruction entry. Changes announce as updates at the next step boundary." | **[DOCUMENTED]** | opencode.ai/v2/docs/api/session (Put instruction entry) |
| V2 Context Epoch: one immutable provider-cache baseline per epoch, stored durably, reused verbatim across restarts; session id doubles as the provider prompt-cache key | **[DOCUMENTED]** | specs/v2/session.md, CONTEXT.md, PR #30789, `runner/llm.ts` |
| V2 has no plugin system-prompt transform hook ("Legacy `experimental.chat.system.transform` … V2 plugins do not yet expose an equivalent hook"); plugin-defined Context Sources deferred | **[DOCUMENTED]** | specs/v2/session.md, PR #30789 |

Why the entry limit matters here: because the Context Builder output must
fit inside one instruction-entry value, **262,144 bytes is the hard
technical ceiling** on the injected block (see `phase2b-retrieval.md` §3;
`phase2b-injection.md` §5).

---

## 3. Physical layout

```
user store  (machine-level)
$XDG_DATA_HOME/owt/                       (default: ~/.local/share/owt/)
  ├── memory.jsonl          user-scope records, mode 0600
  ├── tombstones.jsonl      user-scope forget hashes, mode 0600, append-only
  └── memory.lock           flock target for cross-process writes, mode 0600

project store (inside the repository)
<project-root>/.owt/
  ├── memory.jsonl          project-scope records, mode 0600
  ├── tombstones.jsonl      project-scope forget hashes, mode 0600, append-only
  └── memory.lock           flock target, mode 0600
```

Rules:

1. **One file per scope.** User and project scopes never share a file;
   isolation is structural (P2A §6.2, **[RECOMMENDATION]**).
2. **A scope's records live only in its own file.** A memory's `scope`
   field must match the file it is read from; a well-formed file contains
   only one scope value ([**RECOMMENDATION**], enforced by the loader).
3. **No other files in V1.** No index, no audit log, no sidecar metadata.
   Everything the store needs is in the two files per scope.
4. **Temp files** (`memory.jsonl.tmp*`) may appear transiently during a
   write (§6) and are cleaned on store open.
5. `$XDG_DATA_HOME` is honored when set (Linux/macOS); Windows
   `%APPDATA%/owt` is specified but not implemented in V1 (§9).

---

## 4. Exact memory record schema (V1) — decision D16

**Final resolved field set** (differences from the Phase 2A candidate are
annotated):

| Field | Type | Required | Semantics |
|---|---|---|---|
| `v` | int | yes | Schema version; V1 = `1`. Enables forward migration segmentation. **[RECOMMENDATION]** new |
| `id` | string | yes | Immutable record id: `owt_<unix_millis>_<pid>_<seq>` (seq = per-process counter). Never reused; never changes across rewrites. |
| `key` | string | no | Optional stable handle, `^[a-z0-9][a-z0-9._-]{0,63}$`, normalized to lowercase. Unique among **ACTIVE** records within the scope (§12). |
| `kind` | enum | yes | `fact` \| `preference` (P2A §5, D-superset). |
| `scope` | enum | yes | `user` \| `project` (P2A §6). Must match the containing file. |
| `content` | string | yes | The statement. ≤ 4,096 chars; control characters rejected (§4.2; `phase2b-security.md` §6). |
| `source` | enum | yes | `user` \| `inferred`. V1 writers emit `user` only; `inferred` is reserved (P2A §7). A reader encountering `inferred` tolerates it (forward-compat) and marks it in display. **[RECOMMENDATION]** |
| `status` | enum | yes | `ACTIVE` \| `SUPERSEDED` \| `DELETED`. V1 persists `ACTIVE` and `SUPERSEDED` only; `DELETED` is reserved for V2 soft-delete. Forget in V1 = physical removal (§8, D14). |
| `pinned` | bool | yes | User pinning for injection ordering (P2A §15). Default `false`. **[RECOMMENDATION]** — **resolves the 2A gap**: pinning was referenced by injection design but absent from the record (audit item §8.2 of Phase 2B brief). |
| `created_at` | RFC3339 UTC | yes | Set by the store at first write. |
| `updated_at` | RFC3339 UTC | yes | Set at every write/maintenance touch; equals `created_at` initially. Mechanical only (P2A §9, D10). |
| `session_id` | string | yes | Provenance: session that produced the memory. Phase 4 verified the adapter knows the active session id at command time **[VERIFIED]**; no new OpenCode surface needed. |
| `source_ref` | string | no | Best-effort reference within the session (`msg#3`, step id). **[RECOMMENDATION]** relaxed from P2A §11.2 "required": V1 adapter cannot reliably map a command to a message id (Phase 4 knows only the session id), so it is captured **when identifiable**, otherwise omitted. |
| `quote` | string | no | The user's own words (≤ 256 chars), the evidence. **[RECOMMENDATION]** relaxed from P2A §11.2: required only when the memory quotes conversation text (`/memory remember "user said X" …`); when the user authors content directly in the command, the content *is* the evidence and the quote is omitted. |

Example record (JSON, one line in the file):

```json
{"v":1,"id":"owt_1780000000000_1234_7","key":"lang","kind":"preference",
 "scope":"user","content":"Prefer Rust for new services.",
 "source":"user","status":"ACTIVE","pinned":true,
 "created_at":"2026-09-18T10:00:00Z","updated_at":"2026-09-18T10:00:00Z",
 "session_id":"ses_f4a63109effeiU7GgDsfKzJnSY","source_ref":"msg#3",
 "quote":"I'd rather write new services in Rust."}
```

### 4.1 Audit fixes vs Phase 2A (documented, not silent edits)

| 2A material | Issue found | 2B resolution |
|---|---|---|
| P2A §5 verdict "optional free-form tags stay in the record shape" | Contradicts decision-log **D9** ("no tags in V1") | **No `tags` field in V1.** D9 is authoritative; the P2A §5 sentence is corrected here. Tags return with entities (V3). |
| P2A §15 injection order says "pinned then recency" | Pinning exists as a *concept* but not as a *field* in the P2A §7.3 shape | `pinned: bool` added to the record. |
| P2A §7.3 shape listed `created_at` only | `updated_at` is required by P2A §9/D10 and by lifecycle (superseded_at derivation) | `updated_at` included. |
| P2A §11.2 lists `source_ref` and `quote` as "required" | Adapter cannot always identify them in V1 (Phase 4 evidence) | Both optional, documented above. |
| D8 says "plain append-only JSONL" | Pure append conflicts with hard delete + mutation (this document §5) | Refined (not reversed): record file becomes **rewrite-on-mutation**; tombstones stay append-only. See §5, decision D15. |

### 4.2 Content validation at write time

- `content` length ≤ 4,096 chars; `quote` ≤ 256 chars; `key` ≤ 64 chars.
- Reject NUL (`\u0000`); reject other C0 control characters in `content`
  and `quote` (JSON-legal but hostile to later rendering) —
  **[RECOMMENDATION]**, detail in `phase2b-security.md` §6.
- Valid UTF-8 throughout (files are read/written as UTF-8 JSONL).
- No score fields, no tags, no evidence list, no entity fields (P2A
  §8/D3, D9) — the schema stays exactly as tabled above.

---

## 5. The append-only contradiction — resolved (decision D15)

Phase 2A described JSONL as "append-only/plain file" (D8) while requiring
updates, supersession, hard deletion, and tombstones. **A strictly
append-only record file cannot hard-delete**: the deleted content would
remain on disk, which contradicts forget semantics (privacy) — D14.
Pure append-with-tombstones only *looks* deleted.

Options compared (Phase 2B brief §4.2):

| Criterion | A. Mutable snapshot (random-access rewrite) | B. Append-only event log | C. Append + periodic compaction | **D. Rewrite-on-mutation JSONL** | E. Split mem/tombstone files |
|---|---|---|---|---|---|
| Simplicity | mid | mid | high | **high** | (component of D/E) |
| Crash safety of writes | needs record journaling | good (single appends) | good | **strong** (atomic rename) | — |
| Corruption recovery | hard | replay to last clean line | replay + compact correctness | **read-side: skip bad lines** | — |
| Hard deletion semantics | overwrite in place | impossible without tombstone as *the* representation | tombstone + deferred removal | **immediate, real** | — |
| Tombstone semantics | external | inline | inline | **external, append-only** | — |
| Concurrency | rw-locks | O_APPEND single-writer | lock + safe compact point | **flock around RMW** | — |
| Implementation complexity | low | low | high (compaction engine) | **low** | — |
| Future SQLite migration | manual | replay-based export | replay-based export | **serialize current state → easy** | — |
| File always consistent view for readers | no (torn rows) | yes (row appends; torn tail) | yes | **yes (rename swap)** | — |

**Decision: Option D + E.** The memory record file is *rewritten in full
on every mutation* via temp file + `fsync` + atomic `rename`; tombstones
live in a separate **append-only** file. Reasons:

1. V1 corpus is small (curated, budget-bounded, P2A §15) — a full rewrite
   is microseconds-to-milliseconds; there is no write-frequency problem to
   optimize.
2. Rewrite-on-mutation makes the file **always a complete, current snapshot**:
   the file *is* the state, human-inspectable, diffable, trivially
   exportable, and migration to SQLite later is "serialize current state".
3. Hard delete is **real**: the line is gone from disk after `rename`.
4. Crash safety is the strongest available without dependencies: the main
   file is only ever swapped whole; a crash leaves either the old or the
   new complete file (§6).
5. Pure append (B/C) reintroduces a compaction subsystem V1 does not need
   and keeps deleted content on disk until compaction runs.

**Reversal documentation:** decision-log D8 said "append-only JSONL".
D15 refines this for the *record* file (rewrite-on-mutation) while keeping
tombstones append-only. The `MemoryStore` trait contract (P2A §17) is
unchanged: `{scope, status}` queries returning ordered rows.

---

## 6. Atomicity & crash safety

Write protocol for `memory.jsonl` (every mutation: create, same-key
update/supersede, pin, unpin, forget):

1. Acquire process mutex + cross-process `flock(LOCK_EX)` on
   `memory.lock` (§7).
2. Read current state (or use in-memory cache), apply the mutation in
   memory.
3. Serialize the full new state to `memory.jsonl.tmp.<pid>` in the **same
   directory**.
4. `fsync` the temp file (`File::sync_all`).
5. `rename(memory.jsonl.tmp.<pid> → memory.jsonl)` — atomic on POSIX
   local filesystems.
6. `fsync` the directory (durability of the rename).
7. Release lock; remove temp file if still present (defensive).

Guarantees:

| Scenario | Behavior |
|---|---|
| Crash mid-write | Temp file may exist; `memory.jsonl` still holds the **previous complete state**. No torn record is ever visible. |
| Crash after rename, before dir fsync | On most filesystems the rename may be lost on power loss (file keeps old state) — acceptable: last mutation lost, store consistent. `fsync` on dir narrows this. |
| Power loss | Same as above; the store is never corrupt, only possibly one mutation behind. |
| Partial/truncated `memory.jsonl` | **Cannot happen from our writes** (rename swap). If produced by external tampering, loader skips malformed lines + warns; never crashes (§8). |
| Write fails (disk full, EACCES, EIO) | Mutation aborted, temp file removed, error returned to the command; **store unchanged** (rename never ran). |
| Stale temp files from a crashed earlier process | Removed on store open under the write lock (they are always subsumed: the main file is either old or new, never "half new"). |

**fsync policy:** `fsync` the temp file before every rename, and the
directory after rename. Tombstone appends `fsync` after each append.
This is the minimum for "a completed command survives a crash" on
local disks — **[RECOMMENDATION]**. (Strict group-commit/DEFAULT sync
tuning is unnecessary for a single-user local store.)

**Tombstone append protocol** (`tombstones.jsonl`):

- Open `O_APPEND`, write one complete JSON line per tombstone, `fsync`.
- A torn final line (crash mid-append) is **ignored by readers** (they
  parse complete lines only). Worst case: one forgotten memory could be
  re-rememberable — no integrity loss, no corruption.
- Tombstones are never rewritten, never compacted in V1 (tiny, append-
  only; revisit with SQLite, §15).

---

## 7. Concurrency model

Two cooperating processes (two `owt`/TUI instances) or two sessions within
one process can write the same store.

| Layer | Mechanism |
|---|---|
| Within one process | `std::sync::Mutex` around the store's read-modify-write. flock is per-open-file-description and does not serialize threads sharing an fd — the mutex is mandatory, not optional. |
| Across processes | Advisory `flock(LOCK_EX)` on `memory.lock` around the whole RMW + rename for **any mutation**. |
| Readers | **No lock required.** Because writers only ever swap whole files via rename, a reader opening `memory.jsonl` sees either the old or the new complete file — never a torn one. |
| Both stores | User and project stores are independent; each has its own mutex+lock. |
| Two writers racing | Serialized by flock; the second writer re-reads after acquiring the lock (read-under-lock) so the first's mutation is never lost. |
| Lock contention | Lock file is held only for the duration of a local file rewrite: milliseconds. A stuck holder (crashed process is auto-released by the kernel on fd close) causes at worst a brief unavailability, never a deadlock. |
| Non-cooperating writers | Out of scope: a hostile process with write access to the user's data dir can do anything anyway (`phase2b-security.md` §3). The lock is for our own two processes, which is what local-first concurrency actually means. |

Cross-process lost-update rule: **read-under-lock** — after acquiring
`LOCK_EX`, the writer reloads the file state before applying its mutation.
The in-memory cache (if used) must be invalidated for concurrent processes
(compare file mtime/size, or simply always re-read under the lock for a
small store — cheapest and safest).

---

## 8. Corruption, malformed records, recovery

| Condition | Behavior |
|---|---|
| Line is not valid JSON | Skipped; counted; one aggregate warning ("N malformed records ignored"); store continues. |
| Line is valid JSON but fails schema validation (unknown fields tolerated; bad types/enums rejected) | Same as above: skip + warn. Unknown fields are **tolerated** (forward compatibility), wrong enum values are not. |
| Duplicate `id` in one file | Later occurrence treated as malformed (+warn). Ids are writer-unique; a duplicate means external tampering or a bug. |
| Duplicate ACTIVE `key` in one scope | Invalid per §12; loader warns and keeps both (never silently picks one) — the *writer* is responsible for preventing this. |
| File exists but is empty | Valid: empty corpus. |
| File missing | Valid: empty corpus; created on first write. |
| File is not valid UTF-8 | Refused as a store: memory unavailable + visible warning (see security doc §6). Never partially decoded. |
| File larger than 10 MiB | Refused to load (denial-of-service guard, `phase2b-security.md` §7); memory unavailable + warning. |
| Tombstone file torn final line | Ignored (complete lines only), §6. |

**Recovery discipline:** reads never mutate the store. We never
"auto-repair" a file on read (that would overwrite the very data being
inspected). The only sanctioned mutation paths are explicit commands;
missing/corrupt state degrades to *memory unavailable*, never to *session
failure* (P2A §16; `phase2b-version-resilience.md` §4).

---

## 9. User store vs project store locations

### 9.1 User store (machine-level)

**Decision:** `$XDG_DATA_HOME/owt/` on Linux/macOS; default
`~/.local/share/owt/` when `XDG_DATA_HOME` is unset. Windows:
`%APPDATA%\owt\` (specified for portability, not implemented in V1 —
development platform is Linux).

Rationale:

- XDG data dir is the platform convention for application-owned persistent
  data (docs/`settings` estates go under `~/.config`; *data* under
  `~/.local/share`) **[DOCUMENTED convention, INFERRED fit]**.
- Not the home directory itself (would pollute it); not the
  application/binary directory (breaks installs, needs write perms to the
  install path).
- A distinct `owt` leaf namespace keeps our files unambiguous and
  non-colliding with OpenCode's own `~/.local/share/opencode` data.
- Memory files carry private content → **mode 0600, parent `owt` dir
  mode 0700** (`phase2b-security.md` §2).

### 9.2 Project store

**Decision:** `<project-root>/.owt/` (memory.jsonl + tombstones.jsonl +
memory.lock), with the root defined as the canonical project directory the
adapter already knows (Phase 4 `location.project.directory`).

Why `.owt/` (and not alternatives):

| Candidate | Verdict | Reason |
|---|---|---|
| `<root>/.owt/` | **chosen** | Project-namespaced, unambiguous, invisible to most tooling, survives as the natural home for future project-pinned extras. Not `.owt/memory/*.jsonl` (no need for a subdir in V1). |
| `.opencode/…` | ❌ | **Never**: that is OpenCode's own directory. Writing into it modifies OpenCode's runtime area and violates adapter isolation (AGENTS.md discipline). |
| `<root>/.memory/` | ❌ | Generically named; may collide with other tools. |
| `AGENTS.md`-adjacent tracked file | ❌ | Memory would be indistinguishable from project instructions and would get committed (see §10). |

Project identity is **path-based**: the canonical absolute path of the
project root. Consequences (documented limitations, V1):

- Git worktrees → different directories → different stores (each worktree
  gets its own project memory). A future option is git-common-dir
  identity; not V1.
- Renamed/moved repositories → the store stays in the old path. Memory
  follows the *path*, not the repo; users copy `.owt/` if they want to
  move it. Portability is explicit, not automatic (§10.3).

---

## 10. Git + project memory (decision D20)

**Decision: `.owt/` is git-ignored by default and project memory is
private, single-machine, and machine-specific.** Committing is an
explicit, documented opt-out, not the default.

### 10.1 Should project memory be committed? — tradeoffs

| Property | Committed | Git-ignored (chosen) |
|---|---|---|
| Team sharing of project decisions | ✅ travels with repo, reviewable via PRs | ❌ single machine |
| Survives clone / machine move | ✅ | ❌ (must copy `.owt/`) |
| Git blame/PR review of memory changes | ✅ | ❌ |
| Secrets in memory | ⚠️ risk that a secret gets committed | ✅ much lower blast radius |
| Personal/preference leakage | ⚠️ developer preferences get published with the repo | ✅ private |
| Merge conflicts / JSONL churn | ⚠️ real cost; concurrent edits conflict | ✅ none |
| Repo pollution | ⚠️ every memory edit becomes a diff | ✅ no noise |

### 10.2 Why private-by-default wins for V1

Project memory deliberately overlaps with *private developer perspective
and personal preference* (P2A §12 classifies coding preferences as
memory), and is **not guaranteed secret-free** even though a refusal
pattern exists (`phase2b-security.md` §4). Publishing user-controlled
memory into a shared repo with no review gate is a privacy failure we
cannot walk back. Until V1 has an explicit export/import story and a
review surface, the safe default is: **`.owt/` is local, ignored,
machine-specific** — the same class as `.env` and editor-local state.

### 10.3 Portability

- No automatic cross-machine sync of project memory in V1.
- Portability is **explicit**: copying `<root>/.owt/memory.jsonl` (+
  tombstones) is a supported manual operation; `memory export`/`import`
  is deferred but *enabled* by the file-level format (JSONL is trivially
  transportable). Documented in the roadmap (`phase2b-decision-report.md`
  §5).
- Immutable record fields (id, created_at, session_id) travel unchanged;
  a session_id from another machine remains valid provenance text
  (OpenCode ids are opaque to the store). **[INFERRED]**

---

## 11. Forget + tombstone semantics (precise model) — decision D14 finalized

### 11.1 What the operation does

`forget` (by key or id, `phase2b-retrieval.md` §7):

1. **Resolve** the target record (exact key match among ACTIVE, or exact
   id; ambiguity → error listing candidates, no action).
2. **Remove the line from `memory.jsonl`** via the atomic rewrite
   (§6) — the content and its provenance are erased from the store file.
3. **Append a tombstone** to `tombstones.jsonl`:
   ```json
   {"v":1,"hash":"sha256:61ddc8…","scope":"project","kind":"fact","forgotten_at":"2026-09-18T12:00:00Z"}
   ```
   The tombstone contains the hash only — **no content**, no quote, no
   session id (`phase2b-security.md` §4 reaffirms no content-bearing audit).

### 11.2 What exactly is hashed

- Algorithm: **SHA-256** (std-available via a future dependency? — see
  below; chosen for collision safety and ubiquity).
- Input: the **canonical identity** of the forgotten memory, not raw
  bytes:
  ```
  canonical = kind + "\x00" + scope + "\x00" + normalize(content)
  normalize(content) = trim(), collapse internal runs of whitespace
                       (incl. newlines) to single " "
  hash = SHA-256(canonical)  →  "sha256:<64 hex>"
  ```
- Purpose: two memories are "the same" when their *meaning-identity*
  matches — same kind, same scope, same normalized statement — regardless
  of key, id, or formatting. That is the resurrection case the tombstone
  must block. **[INFERRED]** — a key-based tombstone would miss a
  re-remember of the same statement under a new key.

### 11.3 Where and for how long

- Stored in the scope's `tombstones.jsonl`, append-only, permanent in V1
  (no compaction; tiny; revisit with SQLite in V2, §15).
- **Not** part of the memory record file: deletion must not leave a
  marker inside the file the user asked to be empty of that memory.

### 11.4 What forbids/permits re-addition

| Path | Behavior |
|---|---|
| Re-`remember` of tombstoned content (explicit user command) | **Allowed.** Explicit user intent supersedes the tombstone; the tombstone stays (it is a one-way record, not a ban). V1 has no automatic ingestion, so the tombstone has no silent-resurrection path to block yet. **[RECOMMENDATION]** |
| V2 extraction / import that proposes tombstoned content | **Rejected by the tombstone** — this is exactly what D14 protects against (P2A §13.1:2). |
| Same content re-remembered with a different key | Allowed (tombstone is advisory to the *explicit* path; the new ACTIVE record has its own identity). The V2 dedup engine is what must consult tombstones, not the V1 `remember` command. |

Honest statement: in **V1 the tombstone has little observable effect**
(single-user, explicit-only). It exists because D14's guarantee — "a
forgotten statement can never silently re-enter the injected set" — must
already be true the day extraction/import lands; retrofitting is worse
than carrying the tiny append-only file now.

---

## 12. Key semantics (audit item §8.3 of the Phase 2B brief) — decision D16

| Question | Answer |
|---|---|
| Is `key` optional? | Yes. A record without a key is addressed by `id` only. |
| Format? | `^[a-z0-9][a-z0-9._-]{0,63}$`, **normalized to lowercase at creation** (case-insensitive by construction; mirrors OpenCode's own `InstructionEntry.Key` pattern **[VERIFIED]**). |
| Unique within what? | Unique among **ACTIVE** records of the same `scope`. SUPERSEDED records may share a key with the current ACTIVE (they are the history of that slot). |
| What does `remember <key> <content>` on an existing ACTIVE key do? | **Update/supersession**: old record → `SUPERSEDED`; new ACTIVE record with a new `id` written. Never a silent overwrite — old line retained. (P2A §10.2, §13.3.) |
| What does `remember <key>` when only SUPERSEDED records hold the key do? | Creates a new ACTIVE record reusing the key (the user explicitly restated it). |
| What does `update` mean? | Convenience alias for same-key `remember`, requiring an existing key; same supersession semantics. |
| What does `forget <key>` mean? | Release the key: resolve the ACTIVE record, hard-delete + tombstone (§11). |
| What if key is omitted? | Record is id-addressed; `list`/`show`/`forget` by id work; content is still retrievable in full-corpus reads (V1 injection reads the whole corpus, not by key). |
| Duplicate creation | `remember <key>` when an ACTIVE record already uses the key is *by definition* an update — that is the documented, intentional behavior, not an error. |
| Punctuation/normalization | Trim; lowercase; reject invalid characters with a usage message; no further normalization in V1. |

---

## 13. Status transitions (audit item §8.6) — exact legal set

```text
ACTIVE  ── explicit same-key update (remember/update) ──►  SUPERSEDED
ACTIVE  ── explicit forget ────────────────────────────►  (physical removal + hash tombstone)
ACTIVE  ── V2 machine conflict (future) ───────────────►  CONFLICT      [reserved, not V1]
SUPERSEDED ── nothing. Terminal until the key is restated (new ACTIVE, new id).
DELETED  ── reserved status; V1 never persists DELETED rows (forget removes physically).
```

Rules:

1. **No silent overwrite** (P2A §10.2, D5): every write of a new ACTIVE
   record that displaces an older one first marks the older `SUPERSEDED`
   in the same transaction.
2. `SUPERSEDED → ACTIVE` is **not** allowed (no un-history); the user
   restates via a new record. **[RECOMMENDATION]**
3. `SUPERSEDED` records are excluded from injection (P2A §10.4) and shown
   only by `show`.
4. What `updated_at` means under rewrite-on-mutation: the last write that
   touched the *record* (creation, supersession-marking). It is mechanical
   (P2A §9.1, D10); it never encodes semantic freshness (staleness policy:
   `phase2b-retrieval.md` §6).

---

## 14. JSONL vs SQLite + FTS5 — re-evaluation (Phase 2B brief §13)

P2A deferred SQLite+FTS5 (D8). Re-evaluated here against **actual V1
requirements**:

| Criterion | JSONL (chosen) | SQLite + FTS5 |
|---|---|---|
| Implementation complexity | Low (std only; no Cargo deps — AGENTS.md rule) | Med-high (binding + schema + migrations) |
| Portability | Any filesystem; file = state; trivial to copy | Needs SQLite file care; still portable but binary format |
| Concurrency | flock RMW (milliseconds at V1 scale) | Built-in, robust; overkill for one writer |
| Corruption resistance | Atomic rename + skip-bad-lines; simple invariants | WAL/journal; robust but more failure modes (page damage) |
| Querying | Linear scan; V1 needs only ordered ACTIVE reads | Real queries + FTS5 full-text |
| **Search** | Not needed in V1 (no substring/fuzzy search — `phase2b-retrieval.md` §7) | Would be *unused* in V1 |
| Deletion | Real hard delete (rename) + append tombstone | Delete + tombstone; fine, but adds a schema |
| Inspectability / audit | **cat/grep/diff the file** — strong fit for a local-first tool | Requires sqlite3 tooling |
| Backups | Copy one file | Copy db (+ WAL discipline) |
| Future semantic retrieval | V2 migration needed (see below) | Native path (FTS5 → vector ext) |
| Dependency cost | **Zero** | One Cargo dep + vendored SQLite |
| OpenCode compatibility | None needed (adapter-side) | None needed (adapter-side) |

**Verdict: JSONL remains correct for V1.** The deciding argument is not
"JSONL is simpler" but: **V1 has no search requirement, performs only
ordered whole-corpus reads of a budget-bounded store, and values plain-file
inspectability/backup over query machinery** (D8 evidence stands).
SQLite+FTS5 is justified only when either (a) the corpus exceeds the
injection budget and per-turn retrieval with text ranking arrives (V2+),
or (b) multi-process/write-concurrency or atomic constraints demand a real
DB. Neither is a V1 condition.

If an early deployment shows write/concurrency problems (D8 revisit
condition), the move is to SQLite **behind the existing `MemoryStore`
trait** — no API changes (P2A §17).

---

## 15. Migration path to SQLite (V2, no code now)

When V2 takes SQLite:

1. `MemoryStore` trait stays the only interface (P2A §17).
2. Data model already maps 1:1: records → `memories` table rows; hash
   tombstones → `tombstones` table (or a `forgotten` table); scope stays a
   column.
3. Migration = serialize current JSONL state (fields are already the
   target schema; `v:1` guards drift) into the new store **atomically in
   one write transaction**; keep the JSONL file as a `.bak` until the new
   store's first successful mutation + verification.
4. Super/superseded rows and tombstones migrate as-is (they are already
   expressed: status + tombstones.jsonl).
5. FTS5 content=external table over `content` (and optionally `quote`)
   when text search actually ships — driven by the retrieval requirements
   in `phase2b-retrieval.md` §7, not by storage preference.

---

## 16. Storage test strategy (Phase 5 – must-pass before production)

| Area | Tests |
|---|---|
| Schema | Field validation per enum; unknown-field tolerance; oversized content/quote/key rejection; control-char rejection; scope-must-match-file |
| Atomic writes | Rewrite produces complete file; rename observed atomically; dir fsync called; crash simulation: kill between temp-write/rename/dir-fsync → old or new complete state, no torn file |
| Malformed lines | Non-JSON line → skipped+warned; bad enum → skipped; duplicate id → later skipped; empty file; missing file; non-UTF8 → store unavailable |
| Concurrent writers | Two processes flooding mutations → no lost updates (read-under-lock), final state = all mutations; intra-process threads serialized by mutex |
| Concurrent readers | Reader during writer loop → always a complete file (old or new) |
| Tombstones | Append-only invariant; torn-final-line ignored; hash matches canonical identity (normalization: whitespace variants produce equal hashes); re-remember allowed, V2-dedup-rejection unit-tested at interface level |
| Keys | Normalization (case, trim, invalid chars); ACTIVE-uniqueness; update=supersession; forget releases key; id-addressed ops with no key |
| Status | Legal-transition table enforced; no SUPERSEDED→ACTIVE; no DELETED persisted |
| Corruption recovery | Stale temp cleanup; lock stuck-release; file-too-large guard; store-unavailable degrade path |

---

## ARCHITECTURE STATUS

```text
Storage architecture: resolved. Implementation NOT authorized.
```

Phase 2B is research/design only. The schema, physical layout, write
protocol, and concurrency model above are the contract Phase 5 implements —
no store code exists yet, no Cargo dependencies are added, nothing in the
TUI or OpenCode adapter is modified.