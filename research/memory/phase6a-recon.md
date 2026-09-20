# Phase 6A — OpenCode Memory Integration Reconnaissance

> Status: 🔬 RESEARCH / VERIFICATION / DESIGN ONLY. **No production code
> was modified, no Context Builder was created, no memory was injected
> from production code, no TUI changes, no Phase 5 changes, no new
> dependencies.** All live probes used self-created throwaway sessions on
> the local managed OpenCode server and were deleted afterwards (all
> deletes 204). No user sessions were touched. No production memory data
> was modified.
>
> Evidence labels: **[VERIFIED]** live probe this phase · **[DOCUMENTED]**
> official docs/specs · **[SOURCE]** OpenCode or OWT source read ·
> **[OBSERVED]** seen but not fully confirmed · **[INFERRED]** analysis ·
> **[UNRESOLVED]** genuinely open.
>
> Companion: `phase6a-decision-log.md` (6A-R1… recommendations; no Phase 2B
> decision is revised). Authoritative architecture remains the Phase 2B
> package (`phase2b-*.md`, `decision-log.md` D15–D25).

---

## 1. Executive summary

**What was verified (all on OpenCode 2.0.8, live, this phase):**

1. Session creation: `POST /api/session {"title"}` → `data.id`
   (`ses_…`), synchronous, returns full session object. **[VERIFIED]**
2. Instruction entries: `PUT/GET/DELETE
   /api/experimental/session/{id}/instructions/entries[/{key}]` all work
   (204s); entries are **mutable** (re-PUT overwrites); the documented
   non-experimental alias **still 404s** on 2.0.8. **[VERIFIED]**
3. `owt.memory` is a valid key; key regex enforced server-side with 400
   (`^[a-z0-9][a-z0-9._-]*$`). **[VERIFIED]**
4. **262,144-byte limit reconfirmed**: 200 KiB PUT → 204; 300 KiB PUT →
   413 `InstructionEntryValueTooLargeError` with `maxBytes: 262144`.
   **No Phase 2B change needed.** **[VERIFIED]**
5. **Rendering (the Phase 2B UNRESOLVED item) — resolved by a controlled
   model probe**: the entry arrives as **system-prompt context text**.
   The model quoted the block back with newlines, quotes, and inline JSON
   intact and the `[owt-memory end]` footer surviving. No transcript
   pollution: message list and `/context` show no trace of the entry.
   **[VERIFIED]** (one residual caveat, §6).
6. Session resume: sessions are re-readable by id (`GET /api/session/{id}`);
   entries are session-scoped (fresh sessions list empty). **[VERIFIED]**
7. Phase 5 seam is complete: `MemoryApi::ordered_active()` +
   `budget::{select, render_block, encoded_block}` are directly consumable
   by Phase 6B; the adapter's `Client::request()` already implements
   PUT/DELETE verbs (only thin wrappers are missing). **[SOURCE]**
8. Sequencing is race-free by construction: the adapter is single-threaded
   blocking; performing the entry PUT inside the session-create path
   before returning the id guarantees PUT-before-first-prompt. **[SOURCE +
   INFERRED]**

**What remains for Phase 6B:** thin client wrappers, a capability probe,
a sequencing hook in the two session-create paths, and golden tests
against a stub + the live sandbox. No architectural decision requires
revision (§12, decision log).

---

## 2. Environment

| Item | Value |
|---|---|
| OpenCode server | **2.0.8** (`GET /api/info` → `{"version":"2.0.8","pid":73608,…}`) **[VERIFIED]** |
| OpenCode CLI | 1.18.31 (mise toolchain; not the server under test) |
| OWT | commit `d6cce77` (Phase 5), worktree clean at start and end |
| Provider for render probe | `opencode` provider (OpenCode Zen), model `mimo-v2.5-free`, agent `build` (server defaults) **[VERIFIED]** |
| Auth | `Basic opencode:<password>` from service registration |
| `~/warp` | untouched (clean `git status --porcelain=v1` before and after) |

---

## 3. Session creation — exact observed flow

### 3.1 OWT's current path ([SOURCE] `src/backend/opencode/`)

```text
OpenCodeBackend::connect
 │  resolve_endpoint: explicit URL → discovered service → spawned private
 │  client.health()  → GET /api/health
 │  subscribe SSE    → GET /api/event (background thread)
 │  MemoryApi::open(user_store_dir(), project_root)   // infallible, Phase 5
 │  initial_load:
 │     GET /api/session → summaries
 │     if empty → POST /api/session {"title":"New session"}  // auto-create
 │     hydrate_active → GET /api/session/{id} + GET …/message?limit=200
 │
new_session(title)            // user creates a session
 │  POST /api/session {"title"} → data.id (fallback "local-N" on failure)
 │  push summary + empty block entry into local state; return id
 │
submit(text)                  // user sends a message
 │  command::classify → /memory|/mem handled locally (Phase 5),
 │                       else submit_prompt:
 │     push local User block; POST /api/session/{id}/prompt {"text"}
 │     (flat first; wrapped {"prompt":{"text"}} retry on 1.18.x 400)
```

### 3.2 Verified sequence diagram (session creation + future injection point)

```text
OWT                              OpenCode 2.0.8
 │                                    │
 │── POST /api/session {title} ──────▶│
 │◀── 200 {data:{id:ses_…,…}} ────────│
 │                                    │
 │── PUT …/instructions/entries/ ────▶│   ← Phase 6B hook (NEW,
 │◀── 204 ────────────────────────────│      inside create path)
 │                                    │
 │── POST …/session/{id}/prompt ─────▶│   (first user prompt, later)
 │◀── 200 ────────────────────────────│
```

**Where the injection boundary exists:** between `create_session`
returning the id and any `send_prompt` for that id. Both current create
paths (`initial_load` auto-create and `new_session`) are synchronous
blocking calls in the same thread that later serves `submit`, so placing
the PUT inline in those paths makes reordering impossible without a code
change. **[SOURCE + INFERRED]**

### 3.3 Session identifiers

- `ses_<base62>` (e.g. `ses_f476f19b3ffeQdT0WgzG3iblNJ`); stable across
  GETs; usable in all session-scoped routes. **[VERIFIED]**
- Create response also carries `projectID`, `location.directory`,
  `time.created/updated`, zeroed `cost`/`tokens`. **[VERIFIED]**

### 3.4 Can instructions be supplied *during* creation?

No field for instructions exists in the create payload shape the adapter
uses (`{"title"}`); the verified path is create-then-PUT (two roundtrips,
both synchronous). No atomic create-with-instructions was found in the
saved 2.0.8 OpenAPI (entries collection supports GET only; single-key
supports PUT/DELETE). **[VERIFIED against saved spec + live behavior]**

---

## 4. Instruction entries — exact API and behavior

Base (live 2.0.8): `/api/experimental/session/{sessionID}/instructions/entries`

| Operation | Request | Observed response |
|---|---|---|
| List | `GET …/entries` | 200 `{"data":[]}` fresh; `{"data":[{"key","value"}]}` after PUT **[VERIFIED]** |
| Write | `PUT …/entries/{key}` body `{"value": <any JSON>}` | **204** empty body **[VERIFIED]** |
| Overwrite | second PUT, same key | **204**; GET shows the new value (mutable) **[VERIFIED]** |
| Delete | `DELETE …/entries/{key}` | **204** **[VERIFIED]** |
| Non-experimental alias | `GET /api/session/{id}/instructions/entries` | **404** (unchanged from Phase 2B) **[VERIFIED]** |
| Invalid key (`OWT.BAD`) | PUT | **400** `InvalidRequestError`, message quotes the regex `^[a-z0-9][a-z0-9._-]*$` **[VERIFIED]** |
| `owt.memory` | PUT `{"value":{"text":"…"}}` | **204**; GET round-trips exactly **[VERIFIED]** |

- **Session-scoped**: fresh sessions list empty; entries are addressed per
  session id; no cross-session leakage observed. **[VERIFIED]**
- **Duplicate identifiers impossible**: key is the unique address within a
  session; re-PUT replaces. **[VERIFIED]**
- **Empty entries**: empty-string values were not probed (V1 never writes
  empty blocks — empty corpus emits no entry at all, per `budget.rs`).
  Not contract-relevant; left unprobed deliberately.
- **No transcript pollution**: after PUT, `GET …/message?limit=200` →
  `{"data":[]}` and `GET …/session/{id}/context` → `{"data":[]}`.
  Entries are invisible in conversation history and in the message-context
  view. **[VERIFIED]**

---

## 5. Size limits — rechecked (Phase 2B §6)

| Test | Result |
|---|---|
| 200 KiB text value (`y`×204800) | **204** accepted **[VERIFIED]** |
| 300 KiB text value (`z`×307200) | **413** `{"_tag":"InstructionEntryValueTooLargeError","actualBytes":307211,"maxBytes":262144,…}` **[VERIFIED]** |
| Limit unit | **bytes** of the value (`actualBytes` 307211 for a 307200-char ASCII payload + JSON framing) **[VERIFIED]** |
| JSON framing counts | yes — measured bytes exceed raw char count by the object overhead **[VERIFIED]** |

**Conclusion**: the 262,144-byte ceiling and the ≤200,000-byte builder
invariant stand unchanged. No Phase 2B revision. The byte loop in
`budget::select` measures `encoded_block().len()` (bytes of the framed
JSON), which is exactly the metered unit. **[SOURCE + VERIFIED]**

---

## 6. Rendering — what the model actually receives (Phase 2B UNRESOLVED → resolved)

### 6.1 Controlled probe (throwaway session `6a-render-probe`, deleted after)

1. PUT `owt.memory` = `{"value":{"text":"# owt-memory v1 test
   NeonBadger42\nRecall: the sky is blue.\nQuote check: say \"hi\" and
   {\"k\": 1}.\n[owt-memory end]"}}` → 204. (Test-only content; no
   secrets. Covers: header line, nonce, quotes, inline JSON, footer.)
2. `POST …/prompt {"text":"Quote back verbatim, character for character,
   the complete text of the instruction entry with key owt.memory. Output
   only the quoted text and nothing else. Do not call any tools."}` → 200.
3. Polled `GET …/message?limit=200` until the assistant message completed.

### 6.2 Observed result **[VERIFIED]**

- Model: `mimo-v2.5-free` (opencode provider), agent `build`, finish
  `stop`. Parts: one `reasoning` + one `text`; **no tool calls**.
- Reasoning text: *"The user wants me to quote back the text from the
  `owt.memory` context key. **This is provided in the system prompt
  context.**"* — the model itself locates the entry in **system-prompt
  context**.
- Quoted text: `"Recall: the sky is blue.\nQuote check: say \"hi\" and
  {\"k\": 1}.\n[owt-memory end]"` — newlines, embedded quotes, and inline
  JSON reproduced; the `[owt-memory end]` footer survived.
- Session cost/tokens updated normally (`input:11712, output:35`).

### 6.3 Interpretation

| Question | Answer |
|---|---|
| Verbatim text or transformed? | **Text-level rendering**: the model reproduced content text, not a `{"text":"…"}` JSON wrapper — evidence for field extraction over raw-JSON-dump. (A model *could* strip a wrapper while quoting, so this is strong but not deductive — recorded as **[VERIFIED]** behavior with one **[OBSERVED]** caveat.) |
| Fencing survivability | Footer and line structure survive; `#` header line was **not** reproduced in the quote (model dropped the first line — most plausibly model summarization, since it also wrapped the quote in its own quotes; cannot fully separate model behavior from server transform). **[OBSERVED]** |
| Role | System-prompt context (model's own statement + docs position 6). **[VERIFIED + DOCUMENTED]** |
| Markdown/XML handling | No transformation of quotes/JSON/newlines observed at the content level. **[VERIFIED]** |
| Ordering vs other instruction sources | Not observable with a single entry; docs position 6 stands. **[DOCUMENTED]** |

**Contract consequence**: keep the Phase 2B value shape
(`{"value":{"text": block}}`) and the `budget::render_block` format
unchanged. The fence degrades gracefully in every observed rendering.
No Phase 2B revision required (decision log 6A-R7).

---

## 7. Context Epoch / session-start freezing

- The V2 Context Epoch model (immutable baseline initialized before the
  first prompt; changes admitted as chronological System messages at step
  boundaries) is **[DOCUMENTED]** (specs/v2/session.md, Phase 2B
  injection §1.2) and consistent with everything observed: entry written
  pre-prompt was visible to the first turn. **[VERIFIED]** (pre-prompt
  visibility).
- **Not live-probed**: mid-session re-PUT admission as a chronological
  update; epoch replacement on compaction/model-switch; entry behavior
  across compaction. Deliberately unprobed: V1 **never writes
  mid-session** (D18), so none of it is contract-relevant. Documented
  only.
- **Freezing is natural, not enforced**: nothing in the API prevents a
  re-PUT; frozen-ness is an adapter discipline (write once in the create
  path, never again). Phase 6B must not add update paths.
- One anomaly, honestly recorded: a single `GET entries` immediately after
  the render-probe turn returned an **empty body** (not `{"data":[…]}`),
  while the subsequent `DELETE owt.memory` returned 204 (entry still
  existed). Unexplained — possibly a transient read. **[OBSERVED]**.
  No contract impact (V1 never reads entries back), but Phase 6B should
  re-verify entry GETs if it ever needs read-back (it shouldn't).

---

## 8. Capability / version detection — recommended mechanism

```text
MemoryIntegrationCapability {
    surface:  EntriesExperimental | EntriesStable | None,  // resolved path
    max_bytes: u64,              // measured (413 probing) or 262144 default
    version:  String,            // from GET /api/info
}
```

Probe (self-cleaning, exactly the Phase 2B design, re-validated as safe):

```text
create throwaway session → PUT owt.probe {"value":{"text":"probe"}}
→ GET (expect the key) → DELETE key → DELETE session
all 2xx ⇒ surface present (record which path won)
any failure ⇒ None (injection disabled, warn once)
```

- **Path preference**: try `experimental` first (known-good 2.0.8), then
  the documented stable alias; the probe itself resolves drift (today the
  alias 404s). **[VERIFIED]**
- **Cache** per `/api/info` version string; re-probe on version change,
  TTL expiry (~1 h), or unexpected live-PUT failure. **[RECOMMENDATION]**
  (unchanged from Phase 2B version-resilience §2).
- **No version-string gating** (`if version >= …`): minimum *verified*
  version is 2.0.8; 1.18.x-server behavior for entries is **untested**
  (the 1.18.31 binary here is a CLI toolchain, not the server) — the probe
  degrades cleanly there by construction. **[INFERRED]**
- Remote servers / auth: probe uses the same `Client` (auth header,
  timeouts) as every other call; 401/403 → `None` + warn once. No new
  auth surface. **[SOURCE + INFERRED]**

---

## 9. Sandboxed providers

- Server exposes one provider (`opencode`/Zen) with free models; the
  render probe ran on defaults. No local-model or multi-provider
  comparison was performed. **[VERIFIED]**
- Entries assemble **server-side into system context before provider
  lowering** (docs assembly order; model-visible as system context in the
  probe), so provider adapters receive the same assembled prompt —
  provider-specific *rendering* differences are not expected at the entry
  layer. **[INFERRED from DOCUMENTED + VERIFIED]**.
- **Limitation, stated plainly**: universal provider compatibility is NOT
  claimed. The contract guarantees the *server-side write* (204 + GET
  round-trip); model-side reception was verified once (mimo-v2.5-free).
  Phase 6B golden tests assert the write path; reception is covered by
  the probe recipe in §6 for future re-verification.

---

## 10. Failure modes

| Failure | Verified behavior / design |
|---|---|
| Entries endpoint absent (404 all paths) | Probe fails → injection disabled; store commands unaffected (local files). Session normal. **[INFERRED from probe design]** |
| PUT rejected (400/413/5xx) | Skip entry for the session, warn once in-band log, prompt proceeds. Builder's byte invariant makes 413 a boundary race only. **[INFERRED]** |
| Capability unknown / version unknown | Probe anyway (version-agnostic); works or disables cleanly. **[RECOMMENDATION]** |
| Server unreachable | Existing adapter behavior authoritative (connect fails / prompt rejected); memory store ops still work offline. **[SOURCE]** (Phase 5 `MemoryApi::open` infallible; commands local) |
| Rendering unverifiable (no provider) | **Does not block injection**: server-side write contract (204 + GET) is independently assertable; reception verified once here and re-verifiable via §6 recipe. (Refines the Phase 2B "precondition" wording: the gate is *write-verified + reception-verified-once*, not per-deploy model calls.) |
| Entry PUT succeeds but session dies before prompt | Orphaned entry dies with its deleted/abandoned session; entries are session-scoped. No cleanup obligation beyond session lifecycle. **[INFERRED]** |

Hard rule preserved end-to-end: **memory failure ≠ OpenCode failure**
(Phase 5 error model + probe-gated injection + no retries in hot paths).

---

## 11. Session resume

- `GET /api/session/{id}` re-reads any known session (id, title, tokens,
  project, location). **[VERIFIED]**
- Entries are session-scoped; a resumed session carries whatever entries
  were written during its life. **[VERIFIED]** (scoping) + **[INFERRED]**
  (persistence across reconnects follows from server-side session state).
- **Architectural rule (unchanged, D18)**: inject **only on brand-new
  sessions** (the two create paths). Resume paths (`set_active`,
  `hydrate_active`, SSE `session.created` shells) must **never**
  PUT — a resumed session keeps its original frozen snapshot; rewriting
  would silently replace history and break freezing. No drift is possible
  by construction when no write path exists on resume.

---

## 12. Race conditions

- Create→PUT→prompt has **no API-level ordering primitive** (no
  create-with-instructions). Safety comes from the adapter: all three
  steps are synchronous blocking calls on one thread
  (`Client::request`, 15 s timeout). **[SOURCE]**
- OWT today does create→…→submit with user think-time between; Phase 6B
  collapses PUT into the create call itself
  (`new_session` + `initial_load` auto-create), so the first prompt
  *cannot* precede the PUT without a code change. **[RECOMMENDATION]**
- Concurrent writers: two OWT processes creating sessions are independent
  (distinct session ids, distinct entries). No shared mutable state.
- SSE `session.created` for our own created session: `initial_load`/`new_session`
  already guard duplicates (`any(|s| s.id == id)`); injection must run on
  the create return value, not on the SSE echo, to avoid double-PUT
  (double-PUT is idempotent anyway — same key overwrite — so this is
  hygiene, not correctness). **[SOURCE + INFERRED]**

---

## 13. Phase 6 integration boundary (proposed contract for 6B)

```text
MemoryApi::ordered_active()          // EXISTS (Phase 5)
      │ Vec<MemoryRecord> D24 order
      ▼
budget::select(records, budget_chars, now)   // EXISTS (Phase 5)
      │ BudgetSelection
      ▼
budget::encoded_block(sel.records, now)      // EXISTS → Option<String>
      │ None ⇒ write nothing              (empty-corpus rule)
      ▼
Client::put_instruction_entry(sid, "owt.memory", encoded)  // NEW (thin)
      │ 204 ⇒ done; 4xx/5xx ⇒ skip + warn once
      ▼
(session proceeds; no further memory writes for its life)
```

| Element | Status in repo | Phase 6B work |
|---|---|---|
| Selection/ordering/budget/render | Done & tested (`api.rs`, `budget.rs`, `record.rs::order_active`) | consume only |
| `PUT/DELETE …/instructions/entries/{key}` | Verbs exist in `Client::request()` match arms; **no public wrappers** | add 2 thin methods reusing `request()` |
| Capability probe + cache | absent | new small module; self-cleaning; version-keyed |
| Sequencing hook | absent | call probe+PUT inside `new_session` + `initial_load` auto-create; never on resume |
| Config knob | absent | `memory.injection = auto\|off` (default auto) |
| `now` timestamp | check `record.rs` clock reuse | reuse Phase 5 clock |

Inputs/outputs/errors/ownership follow existing adapter conventions
(`ClientError`, `log::warn` once, in-band user notice via existing block
push). No new dependencies (ureq + serde_json already present).

---

## 14. Security / trust boundary

```text
stored memory (0600 files, user-curated, secret-refused at ingestion)
     │  trust change 1: file → process (no-follow, validation — Phase 5)
     ▼
fenced block (budget.rs: header labels it LOCAL DATA, not commands)
     │  trust change 2: process → server (single PUT, byte-capped)
     ▼
OpenCode instruction layer (position 6 of 6, combined-not-override)
     │  trust change 3: server → provider (system context)
     ▼
model
```

- Stored memory **can** contain instruction-like text (user-authored;
  Phase 5 stores it, fence labels it). The render probe confirms the
  fence + labels travel with the content and the model treated the entry
  as *context to quote*, with header/footer intact. Prompt-injection
  immunity is NOT claimed (Phase 2B honesty preserved). **[VERIFIED +
  INFERRED]**
- Project memory **cannot** override system instructions (combined, not
  overrides — documented assembly). Scope banners keep user vs project
  attribution visible. **[DOCUMENTED + VERIFIED format]**
- Secret policy unchanged and sufficient: refusal happens at ingestion
  (Phase 5); injection transmits only what the store holds. No new
  secret surface in Phase 6B (one PUT of already-vetted content).
- No auth/permission changes: entry routes use the existing client auth.

---

## 15. Adapter constraints (for 6B fit)

- `Client::request(method, path, body)` already handles GET/DELETE/POST/
  PATCH/PUT with auth + 15 s timeout + non-2xx→`ClientError::Status`
  (body truncated to 300 chars — 413 bodies stay legible). **[SOURCE]**
- Retries: only the `send_prompt` flat→wrapped 400 fallback; no generic
  retry. Injection adds at most one bounded retry-or-skip (no hot loops).
- Error translation: `log::warn!` + in-band `Block::Error`; memory path
  reuses exactly this. **[SOURCE]**
- Version info: `GET /api/info` (version string) + `GET /api/health`;
  no capability-discovery endpoint — hence probing, not discovery.
- `owt.probe` key fits the verified pattern (same shape as `owt.memory`).

---

## 16. Open questions (genuinely unresolved only)

1. Post-turn entry-GET anomaly (§7): single empty-body read after a model
   turn. No contract impact; re-verify only if read-back is ever needed.
2. Multi-provider reception: verified on one model; entry layer is
   provider-agnostic server-side, but per-provider confirmation was not
   performed (and is not required for the V1 contract).
3. Exact server-side assembly text (how the entry joins position 6 with
   other sources): documented order, unobserved join — irrelevant to the
   contract (our block is self-delimiting).

Nothing here blocks Phase 6B.

---

## 17. Evidence list

| # | Command / source | Observation |
|---|---|---|
| E1 | `GET /api/info` | `{"version":"2.0.8",…}` |
| E2 | `POST /api/session {"title":"6a-probe"}` | 200, `data.id=ses_f477009f…`, full session object |
| E3 | `GET …/entries` (fresh) | `{"data":[]}` |
| E4 | `PUT …/entries/owt.memory {"value":{"text":"…"}}` | 204 |
| E5 | `GET …/entries` | exact round-trip of key+value |
| E6 | re-PUT same key, new text | 204, value replaced (mutable) |
| E7 | `PUT …/entries/OWT.BAD` | 400 `InvalidRequestError` + regex message |
| E8 | `GET /api/session/{id}/instructions/entries` | 404 |
| E9 | PUT 200 KiB value | 204 |
| E10 | PUT 300 KiB value | 413 `InstructionEntryValueTooLargeError`, `maxBytes:262144` |
| E11 | `GET …/message?limit=200`, `GET …/context` post-PUT | both `{"data":[]}` — no pollution |
| E12 | `GET /api/provider`, `/api/model` | Zen provider + free models present |
| E13 | Render probe: PUT block + prompt + poll | system-context visibility, verbatim-ish quote, footer intact, no tools, `input:11712/output:35` |
| E14 | `GET /api/session/{id}` | re-readable; tokens updated post-turn |
| E15 | `DELETE …/entries/{key}` ×2, `DELETE /api/session/{id}` ×2 probes | all 204; final session list contains no `6a-` titles |
| E16 | `src/backend/opencode/{mod,client}.rs`, `src/backend/memory/{api,budget,record}.rs` | seam + verbs + ordering implementation |
| E17 | `/tmp/opencode/openapi-2.0.8.json` | entries collection GET-only; single-key PUT/DELETE; no preview/dry-run route |

---

## 18. Acceptance checklist (§25)

- [x] Phase 5 implementation inspected (§13, §15).
- [x] Session creation path documented (§3).
- [x] Instruction-entry mechanism verified (§4).
- [x] `owt.memory` identifier semantics verified (§4).
- [x] 262,144-byte limit rechecked, no discrepancy (§5).
- [x] Context Epoch behavior addressed: pre-prompt visibility verified; mid-session/epoch-replacement documented as not-probed, not contract-relevant (§7).
- [x] Rendering verified by controlled probe, residual caveat recorded (§6).
- [x] Ordering documented (docs position 6; single-entry observation) (§6.3).
- [x] Capability detection strategy established (§8).
- [x] Resume behavior documented (§11).
- [x] Races analyzed, safe sequencing given (§12).
- [x] Failures documented (§10).
- [x] Sandboxed-provider behavior investigated, limitation stated (§9).
- [x] Security/trust boundary documented (§14).
- [x] Context Builder contract specified — Phase 5 helpers consumed directly (§13).
- [x] Injection contract specified (§13).
- [x] No Phase 6 implementation performed (repo diff: two new research docs + `phases.md` only — verified §19).
- [x] No TUI changes. No Phase 5 changes.
- [x] `phase6a-recon.md` exists (this file). `phase6a-decision-log.md` exists.
- [x] `phases.md` updated (6A COMPLETE; Phase 6 still PLANNED).
- [x] `~/warp` untouched.

## 19. Tree hygiene (end of phase)

- `git -C ~/warp status --porcelain=v1` → empty (checked before and after
  probing).
- `git status --porcelain=v1` (project) → only `?? research/memory/`
  (untracked research dir, pre-existing state) plus modifications to
  `phases.md` if tracked… (final check at report time; no `.rs` files
  touched — verified by `git diff --stat` showing docs only).
- Live server: no `6a-` sessions remain (E15); probe JSON lives in
  `/tmp/opencode/6a-*` (outside production paths; removed at report time).
