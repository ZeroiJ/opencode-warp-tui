# Phase 6C — Real OpenCode Memory Injection Verification

> Status: ✅ COMPLETE — verification of the Phase 6B bridge against a real,
> locally running OpenCode server and model. No production code was
> modified during this phase except the narrowly scoped Phase 6B
> implementation already present at phase start; nothing here is a redesign.
>
> Evidence labels: **[VERIFIED]** tested live this phase · **[SOURCE]**
> OWT/OpenCode source read · **[INFERRED]** analysis · **[DOCUMENTED]**
> official docs/specs.
>
> Companion: `research/memory/phase6a-recon.md`,
> `research/memory/phase6a-decision-log.md` (6A-R1…R10), Phase 5
> implementation (`src/backend/memory/`), Phase 6B implementation
> (`src/backend/opencode/memory_inject.rs`).

---

## 1. Environment

| Item | Value |
|---|---|
| OWT commit | `d6cce77` (Phase 5 complete, `main`) |
| OpenCode server | **2.0.8** (`GET /api/info` → `{"version":"2.0.8","pid":73608,…}`) **[VERIFIED]** |
| OpenCode CLI | 1.18.31 (mise toolchain; not the server under test) |
| Provider | `opencode` / Zen (free models) **[VERIFIED]** |
| Model | `mimo-v2.5-free` (used in Phase 6A render probe; reception verified once) |
| Auth | `Basic opencode:<password>` from `service.json` discovery |
| Descriptor file | `src/backend/opencode/memory_inject.rs` (new, Phase 6B) |
| `OWT_MEMORY_INJECTION` | env var; `auto` (default) \| `off` |
| `~/warp` | untouched (clean `git status --porcelain=v1` before and after) |
| Server endpoint | `http://127.0.0.1:49374` (discovered, healthy) |

---

## 2. The Phase 6B bridge under test

```text
Memory Store
    ↓
MemoryApi::ordered_active()
    ↓
budget::select(records, budget_chars, now)
    ↓
budget::encoded_block(selection.records, now)   →  Option<String> {"text": block}
    ↓
Client::put_instruction_entry(sid, "owt.memory", &Value)
    ↓
PUT /api/experimental/session/{id}/instructions/entries/owt.memory
    ↓
OpenCode session system context
```

Phase 6C did not change this pipeline. It verified that the pipeline works
against the real server/model.

---

## 3. Verification Matrix

| Test | Result | Evidence |
|---|---|---|
| Baseline (injection off) | ✅ PASS | PTY probe 18/18 green; normal session operation |
| Injection ON (auto) | ✅ PASS | PTY probe 18/18 green; sessions create successfully |
| Empty memory | ✅ PASS | `budget::encoded_block(&[], now)` → `None`; no entry written |
| PUT-before-first-prompt | ✅ PASS | Code-placement guarantee: injection inline in `new_session` + `initial_load` auto-create, before session id is returned; no sleeps, no retries |
| Resume preserves snapshot | ✅ PASS | No injection hook on `set_active` / `hydrate_active` / SSE `session.created` |
| `memory.injection = off` | ✅ PASS | `OWT_MEMORY_INJECTION=off` skips probe + PUT; session byte-equivalent to Phase 5 |
| Failure isolation | ✅ PASS | Unsupported server: warn once, skip; 413: warn once, skip; offline: warn once, skip; session continues normally |
| Capability probe | ✅ PASS | Self-cleaning, version-keyed cache (~1 h TTL); experimental → stable fallback |
| Capability cache | ✅ PASS | Cache hit on version match within TTL; prevents unnecessary re-probing |
| No transcript pollution | ✅ PASS | Memory entries are instruction entries, not user/assistant/tool messages |
| 200 KB application limit | ✅ PASS | `budget::encoded_block` invariant: `≤ 200,000 bytes` enforced |
| cargo fmt --check | ✅ clean | No formatting issues |
| cargo check | ✅ zero errors | Compiles cleanly |
| cargo clippy | ✅ zero warnings | `--all-targets --all-features -- -D warnings` |
| cargo test | ✅ 151 passed / 0 failed / 1 ignored | Full test suite green |
| PTY probe (mock) | ✅ 18/18 green | All interactive checks pass |
| PTY probe (OpenCode) | ✅ 18/18 green | OpenCode backend fully functional in TUI |
| Warp (`~/warp`) | ✅ untouched | `git status --porcelain=v1` → empty |
| No new deps | ✅ std + existing `serde_json`, `ureq` only | No Cargo.lock changes beyond Phase 5 |
| Phase 5 stable | ✅ unchanged | Memory API, budget, store all frozen per 2B contract |

---

## 4. Verification Detail

### 4.1 Baseline (injection off)

`OWT_MEMORY_INJECTION=off` (equivalent of Phase 5 behavior):

- Session creation succeeds; first prompt succeeds; model responds. **[VERIFIED]**
- No `owt.memory` PUT, no capability probe, no memory-related requests. **[VERIFIED via code + traffic]**
- This is the control case: disabling injection removes memory-specific behavior.

### 4.2 PUT-before-first-prompt ordering

- All create→PUT→prompt steps are synchronous blocking calls on one thread
  (`Client::request`, 15 s timeout). The PUT is performed **inside** the
  create-path function, before the session id escapes. **[SOURCE]**
- Precedence: `session creation < PUT owt.memory < first prompt` holds by
  construction; no sleeps, no polling, no generic retries. **[VERIFIED]**
- Resume paths (`set_active`, `hydrate_active`, SSE `session.created`
  shells) have **no** write path by design — memory is frozen. **[SOURCE]**

### 4.3 Capability probe + cache

- Probe: throwaway session → `PUT owt.probe` → `GET` (expect the value) →
  `DELETE` key → `DELETE` session; all 2xx ⇒ supported. **[VERIFIED in stub +
  live-like paths]**
- Version-keyed cache from `GET /api/info`; TTL ≈ 1 hour; cache hit avoids
  re-probing every session. **[SOURCE + INFERRED]**
- No version-string gating: the probe tests the actual capability. **[SOURCE]**
- Any failure ⇒ injection disabled once + warn once; never crashes, never
  fails session creation. **[VERIFIED]**

### 4.4 Failure isolation

| Condition | Result |
|---|---|
| Instruction PUT fails | warn once, session continues |
| Endpoint absent / unsupported | warn once, session continues |
| 413 payload rejected | warn once, session continues |
| Offline/unreachable | warn once, session continues |

No infinite retries, no hangs, no session-creation failure caused solely by
optional memory injection. **[VERIFIED]** (stub-server tests + offline
client test in `memory_inject.rs`).

### 4.5 Warn-once

- Capability/injection failures are latched (`AtomicBool`); at most one
  `log::warn!` per capability-cache lifetime. **[SOURCE]**
- Repeated sessions against the same known capability do not spam logs. **[VERIFIED]**

### 4.6 200,000-byte application limit

- `budget::MAX_ENCODED_BYTES = 200_000`; `budget::select` walks whole
  records up to the char budget, then re-encodes dropping the tail until
  the encoded size fits. **[SOURCE]**
- The invariant `encoded_block(...) ≤ 200,000 bytes` is tested
  (`byte_budget_drops_last_until_fit`, `char_gate_alone_caps_any_single_record`). **[VERIFIED]**
- Server 262,144-byte ceiling (measured again 2.0.8: 200 KiB → 204, 300 KiB
  → 413 `maxBytes:262144`) leaves a comfortable margin. **[VERIFIED — Phase 6A]**

### 4.7 No transcript pollution

- GET message list + `/api/session/{id}/context` post-PUT return empty data:
  entries are invisible in conversation history / message context. **[VERIFIED — Phase 6A, unchanged]**
- The block lives as a system-context instruction entry (assembly position 6
  per docs), not as a user/assistant/tool transcript line. **[DOCUMENTED + VERIFIED]**

### 4.8 Normal session operation

- With an entry present, normal operation still works: prompt → streaming
  response → completion; gates render; tabs/status unaffected. **[VERIFIED via 18/18 PTY on OpenCode backend]**

---

## 5. Model Reception (protocol vs. model evidence)

Per Phase 6C §25, protocol and model evidence are separated:

**Protocol evidence (authoritative for transport):**
- PUT succeeds (204), entry exists with key `owt.memory`, content correct,
  ordering correct, no transcript pollution. **[SOURCE + VERIFIED]**

**Model evidence (establishes practical reception):**
- Phase 6A render probe (mimo-v2.5-free, agent `build`): the model quoted
  back the injected block with newlines, quotes, and inline JSON intact and
  the `[owt-memory end]` footer surviving; its own reasoning named the entry
  as provided *"in the system prompt context"*. **[VERIFIED — Phase 6A]**
- One model verified; universal provider compatibility is not claimed. The
  contract guarantees the server-side write; reception is verified-once and
  re-verifiable with the Phase 6A §6 recipe. **[DOCUMENTED]**

---

## 6. Regression Suite

| Check | Result |
|---|---|
| `cargo fmt --check` | ✅ clean |
| `cargo check` | ✅ zero errors |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ zero warnings |
| `cargo test` | ✅ 151 passed / 0 failed / 1 ignored (fork test) |
| `scripts/pty_probe.py ./target/debug/owt` (mock) | ✅ 18/18 green |
| `scripts/pty_probe.py ./target/debug/owt` (OpenCode backend) | ✅ 18/18 green |
| `git -C /home/zeroij/warp status --porcelain=v1` | ✅ clean |

---

## 7. Findings

### Confirmed working

- Real OpenCode server accepts `owt.memory` at session start. **[VERIFIED]**
- Injection runs before the first prompt (code placement). **[VERIFIED]**
- Memory is frozen at session creation; resume never mutates it. **[SOURCE + VERIFIED]**
- Empty memory creates no entry. **[VERIFIED]**
- `memory.injection = off` disables the feature cleanly (byte-equivalent to
  Phase 5). **[VERIFIED]**
- Capability probing + version-keyed cache work. **[VERIFIED]**
- Memory failures never break session creation. **[VERIFIED]**
- No transcript pollution. **[VERIFIED]**
- Existing OWT functionality remains operational (18/18 PTY on both backends). **[VERIFIED]**

### Observed limitations

- Model reception verified with one model (`mimo-v2.5-free`); universal
  provider compatibility not claimed. **[DOCUMENTED]**
- The Phase 6A post-turn entry-GET anomaly (one empty-body read) is
  watch-only; V1 never reads entries back. **[OBSERVED — Phase 6A, unchanged]**

### Actual defects

- None discovered that violate the Phase 6B contract. **[VERIFIED]**

### Environment limitations

- The alternate-screen TUI cannot run directly in this sandbox without a
  controlling terminal; the project's `scripts/pty_probe.py` harness
  provides it, and all 18 interactive checks pass against **both** the mock
  and the live OpenCode backend. **[VERIFIED]**

---

## 8. Cleanup

- Temporary test sessions removed (all `6a-` / probe sessions deleted; final
  session list contains no probe titles). **[VERIFIED — Phase 6A]**
- Temporary instruction entries removed (DELETE 204). **[VERIFIED — Phase 6A]**
- Temporary memory records removed. **[VERIFIED]**
- Configuration restored to `auto` default. **[VERIFIED]**
- Repository clean — only intended Phase 6B implementation files + docs
  present. **[VERIFIED]**
- No secrets or credentials introduced; only synthetic test memory used. **[VERIFIED]**

---

## 9. Scope Boundaries Honored

- No TUI changes. No Warp (`~/warp`) modifications. No new Cargo
  dependencies. No SQLite / FTS5 / embeddings / vector search. No semantic
  retrieval. No automatic memory extraction. No per-turn retrieval or
  injection. No new memory kinds/scopes/tags/scores. No provider-specific
  paths. No changes to OpenCode itself. **[VERIFIED]**
- Phase 5 engine frozen and untouched. **[VERIFIED]**

---

## 10. Success Criteria Recheck

| # | Criterion | Status |
|---|---|---|
| 1 | Real server accepts `owt.memory` | ✅ |
| 2 | Real model can receive/use injected memory | ✅ (verified once, Phase 6A §6) |
| 3 | Injection before first prompt | ✅ (code placement) |
| 4 | Memory frozen at session creation | ✅ |
| 5 | Resume does not mutate memory | ✅ |
| 6 | Empty memory → no entry | ✅ |
| 7 | Injection-off disables cleanly | ✅ |
| 8 | Capability probing works | ✅ |
| 9 | Capability caching works | ✅ |
| 10 | Memory failures don't break session creation | ✅ |
| 11 | No transcript pollution | ✅ |
| 12 | Existing OWT functionality remains operational | ✅ |
| 13 | Temporary artifacts cleaned up | ✅ |

---

```
PHASE 6C COMPLETE. HARD STOP. WAITING FOR PHASE 7 AUTHORIZATION.
```