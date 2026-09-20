PHASE 6B COMPLETE. HARD STOP. WAITING FOR PHASE 6C AUTHORIZATION.

## Implementation Summary

### Files Changed (5 existing)
- `src/backend/opencode/client.rs` — added `info()`, `delete_session()`, `put_instruction_entry()`, `get_instruction_entry()`, `delete_instruction_entry()` thin wrappers reusing existing `request()`; fixed 204/empty-body JSON parse edge
- `src/backend/opencode/config.rs` — added `MemoryInjection` enum (`Auto`/`Off`), `OpenCodeConfig.injection` field, `ENV_*` constants; `from_env()` reads `OWT_MEMORY_INJECTION`
- `src/backend/opencode/mod.rs` — added `MemoryInjection`, `Injector` fields to `OpenCodeBackend`; wired injection into both create-session paths (`new_session` + `initial_load` auto-create); session-resume path has NO injection hook
- `src/backend/opencode/memory_inject.rs` **new** — complete injection module: capability probe (self-cleaning, version-keyed cache TTL ~1 h), `build_entry()`, `inject_new_session()` with all failure-isolation modes (empty corpus → skip, 413 → warn-skip, unsupported → warn-skip, offline → skip), warn-once latches, D24 ordering via existing `ordered_active()`/`select()`/`encoded_block()`, stub test server in test module
- `src/backend/opencode/memory_inject.rs` — test module: 13 tests covering empty corpus, correct key+shape, unsupported server, 413 low limit without failure, offline never-fails, probe self-cleaning, context builder D24+budget, env gate parsing, plus 4 parameterized `memory_with_fact("name")` helpers

### Files Added (1 new)
- `src/backend/opencode/memory_inject.rs` — the complete Phase 6B adapter

### What Was Implemented

**Memory path** (straight line, no detours):

```
create session (new_session or initial_load auto-create)
    │
    ├─► read config `memory.injection` (default: auto)
    │
    ├─► if auto: capability probe → version-keyed cache (~1 h TTL)
    │     throwaway session → PUT owt.probe → GET → DELETE key → DELETE session
    │     experimental first (known-good on 2.0.8); stable alias fallback
    │     cache hit on version match within TTL → reuse; otherwise re-probe
    │
    ├─► MemoryApi::ordered_active() → D24 order
    │        ↓
    │   budget::select(records, DEFAULT_BUDGET_CHARS=12_000, now)
    │        ↓
    │   budget::encoded_block(selected, now) → {"text": block} Option<String>
    │        ↓
    │   PUT /api/experimental/session/{sid}/instructions/entries/owt.memory
    │        ↓ 204 ⇒ done; 413 ⇒ warn-skip; other failure ⇒ warn-skip
    │
    └─► session proceeds; first prompt CANNOT precede the PUT-or-skip
        (single-threaded blocking adapter; code placement is the guarantee)
```

**Configuration behavior**:
- Default: `auto` → attempt injection when server supports it
- `off` → skip probe + PUT entirely; session behaves byte-equivalent to Phase 5

**Session lifecycle guarantees**:
- **New session** (both `new_session` and `initial_load` auto-create): injection runs inline before returning the session id
- **Resume** (`set_active`, `hydrate_active`, SSE `session.created` echoes): **never** calls injector — snapshot stays frozen from original creation
- **PUT-before-first-prompt**: guaranteed by code placement in the synchronous create path; no retries, no sleeps, no protocol-level ordering primitive needed

**Capability probe** (per Phase 6A R2):
- Self-cleaning throwaway session path, version-keyed cache (~1 h TTL per Phase 6A)
- Experimental endpoint first, then stable alias fallback
- Any failure ⇒ injection disabled + warn once; never crashes, never fails session creation
- Distinguishes `supported` / `unsupported`; expires on version change

**Failure isolation** (per Phase 6A R10 / §19):
- Empty corpus → no entry written
- Unsupported server → warn once, injection skipped, session proceeds normally
- 413 size rejection → warn once, injection skipped, session proceeds normally
- Offline/unreachable → warn once, injection skipped, session proceeds normally
- Any memory failure → warn once, continue OpenCode session (never fail creation)

**Tests**: 151 pass, 0 fail, 1 ignored (fork test); all PTY harness checks (18/18) still pass

### Verification Results

| Check | Result |
|---|---|
| `cargo fmt --check` | ✅ clean |
| `cargo check` | ✅ zero errors |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ zero warnings |
| `cargo test` | ✅ 151 passed / 0 failed / 1 ignored |
| `scripts/pty_probe.py ./target/debug/owt` | ✅ 18/18 checks green |
| `git -C /home/zeroij/warp status --porcelain=v1` | ✅ clean (untouched) |

### Diff Hygiene

| Check | Result |
|---|---|
| TUI changes | ✅ none |
| Warp (`~/warp`) modifications | ✅ none |
| New Cargo dependencies | ✅ none (only std + existing `serde_json`, `ureq`) |
| New memory architecture | ✅ none (Phase 5 API consumed directly) |
| SQLite / embeddings / FTS5 / vector DB | ✅ absent |
| Secret refusal weakened | ✅ unchanged |
| New TUI memory UX | ✅ absent |
| Automatic memory extraction | ✅ absent |
| Provider-specific paths | ✅ absent |

### Remaining Issues (none — genuinely)

The implementation is complete. The only open question is Phase 6C authorization.

**The final line of this report must be:**

```
PHASE 6B COMPLETE. HARD STOP. WAITING FOR PHASE 6C AUTHORIZATION.
```