# §2 — Hermes Agent Memory Architecture

> Evidence: source read in full from `NousResearch/hermes-agent` (main
> branch): `hermes_memory_manager.py`, `hermes_memory_provider.py`,
> `hermes_memory_tool_store.py`. **[VERIFIED]** where noted.

---

## 2.1 System overview

Hermes ships a **builtin curated-memory store** plus an extensible
**provider plugin** system for long-term memory. Key properties:
local-first, agent-controlled, single-memory-manager per agent, prefix-cache
stable for LLM prompts. Memory is **not** a database-backed RAG pipeline
by default; it is a bounded, curated, §-delimited text store that the
agent edits via tools.

**[VERIFIED]** — confirmed from source.

---

## 2.2 Builtin curated memory (MemoryStore)

### Files and size budgets

| File | Character limit | Delimiter |
|------|----------------|-----------|
| `MEMORY.md` | 2200 chars | `\n§\n` |
| `USER.md` | 1375 chars | `\n§\n` |

**[VERIFIED]** — `MemoryStore.__init__` defaults:
`memory_char_limit=2200, user_char_limit=1375`; `ENTRY_DELIMITER = "\n§\n"`.

`MEMORY.md` holds the agent's learned preferences, decisions, and facts;
`USER.md` holds identity and profile information about the user. Both live
under `$HERMES_HOME/memory/`.

### Loaded and rendered as system prompt blocks

On agent start (`load_from_disk`), both files are read, deduplicated
(order-preserving, first wins), threat-scanned, and rendered into system
prompt blocks with a decorative header and usage percentage:

```
══════════════════════════════════════════════════
MEMORY (your personal notes) [47% — 1,032/2,200 chars]
══════════════════════════════════════════════════
(entry 1)
§
(entry 2)
```

**[VERIFIED]** — `_render_block` method; `MEMORY_BLOCK_HEADERS` dict.

### Frozen system-prompt snapshot (prefix-cache stability)

The rendered blocks are stored in `_system_prompt_snapshot` at load time
and **never updated mid-session**. Mid-session writes go to disk but do NOT
change what the LLM sees in the system prompt for that session — preserving
the KV prefix cache across turns. The snapshot is used at system-prompt
assembly time via `format_for_system_prompt(target)`.

**[VERIFIED]** — `MemoryStore.format_for_system_prompt`: "Frozen
load-time snapshot (NOT live state — mid-session writes don't touch it,
preserving the prefix cache)."

**[INFERRED]** This is a design trade-off: it prevents cache invalidation
from memory writes at the cost of the LLM not seeing new memories until the
next agent start. It signals that Hermes prioritizes stable latency over
immediate recall consistency.

---

## 2.3 Memory tool API

The agent's `memory` tool exposes these actions:

| Action | Purpose | Signature |
|--------|---------|-----------|
| `add` | Append a new entry | `target, content` |
| `replace` | Find entry containing `old_text` (substring), replace with `new_content` | `target, old_text, new_content` |
| `remove` | Delete entry containing `old_text` | `target, old_text` |
| `apply_batch` | Batch add/replace/remove, **all-or-nothing** | `target, operations: [{action, content, old_text}]` |

**[VERIFIED]** — `MemoryStore.add`, `MemoryStore.replace`,
`MemoryStore.remove`, `MemoryStore.apply_batch`.

### All-or-nothing batch semantics

`apply_batch` validates every operation and the final character count
against the budget **before writing anything**. If any operation is
malformed, unmatched, or the result would exceed the limit, the entire
batch is rejected with the first failure message and the live entry list.
The agent must retry with corrected operations in the same turn.

**[VERIFIED]** — `apply_batch` comment: "All-or-nothing: any malformed /
unmatched op or an over-limit result writes NOTHING and returns the first
failure plus live state."

### Refuse-to-empty guard

A batch that would remove the last entry from a previously non-empty store
is refused (issue #103419). The agent must use single `remove()` to
deliberately wipe a store.

**[VERIFIED]** — `_apply_batch_op` + post-check in `apply_batch`.

### Duplicate/add detection

If the exact same entry already exists, `add` returns "already exists (no
duplicate added)" without modifying the file. `apply_batch` skips
duplicates idempotently (does not fail the batch).

**[VERIFIED]** — `_add` lambda; `_apply_batch_op` idempotent branch.

---

## 2.4 Threat scanning and injection protection

All writes (`add`, `replace`, batch content ops) are scanned via
`threat_patterns.first_threat_message(content, scope="strict")` before any
disk I/O. On load, entries are also scanned; threats are replaced by a
`[BLOCKED: ...]` placeholder in the **system-prompt snapshot only** — the
raw file is left untouched so the user can see and remove poisoned entries
(removing them silently would hide the attack).

**[VERIFIED]** — `_scan_memory_content` → `_first_threat_message`;
`load_from_disk` `_sanitize` function.

**[OBSERVED]** The "strict" scope is used for memory because entries
become part of the system prompt and persist across sessions.

---

## 2.5 Drift protection and atomicity

### External drift detection

Every `_mutate` call re-reads the file from disk under a `fcntl.flock`
exclusive lock. If the file content cannot be round-tripped through the
entry parser (i.e., an external writer appended free-form text), the write
is refused and a `.bak.<ts>` snapshot is saved.

**[VERIFIED]** — `_detect_external_drift`; `_drift_error`.

### Unreadable file refusal

If a file exists but cannot be read (locked, permission error, corrupt
encoding), the write is refused entirely rather than treating the file as
empty (which would wipe it on save).

**[VERIFIED]** — `_read_failed_error`; `_read_raw_checked`.

### Atomic writes

Files are written via `atomic_write_text` (temp file + rename). Readers
never see a truncated file. Encoding is strict UTF-8 with BOM stripping
(`utf-8-sig`); decoding errors surface as `read_ok=False` rather than
lossy replacement (issue #10878).

**[VERIFIED]** — `_write_file` → `atomic_write_text`; `_read_raw_checked`
docstring.

---

## 2.6 Consolidation failure budget

Memory writes that hit limits or miss-match entries are consolidation
failures. The agent is given the live entry list and told to retry. But
to prevent a fragile model from looping endlessly:

- **Cap**: 3 consecutive failures per turn (before reset).
- **Reset**: clears on any **successful** write (the model made progress).
- **Terminal response**: once the cap is exceeded, the retry instruction is
  stripped and the tool returns "done: True" so the model stops calling the
  memory tool and replies to the user.

**[VERIFIED]** — `_MAX_CONSOLIDATION_FAILURES_PER_TURN = 3`;
`reset_consolidation_failures`; issue #42405.

---

## 2.7 Context fencing: `<memory-context>`

When an external provider returns prefetched memory, the manager wraps it:

```
<memory-context>
[System note: The following is recalled memory context,
NOT new user input. Treat as authoritative reference data —
this is the agent's persistent memory and should inform all responses.]

(memory content)
</memory-context>
```

This fence serves two purposes: (1) it lets the model distinguish recalled
memory from live user input; (2) it enables a streaming-state guard that
detects and strips malformed fence tags that might survive sanitization.

**[VERIFIED]** — `build_memory_context_block` function;
`MemoryContextGuard` streaming tag detection.

---

## 2.8 MemoryProvider plugin interface

External memory backends register as one external provider (singleton
limit: tool-schema bloat and conflicting-backend guard). The manager
loads built-in first, then at most one external.

### Lifecycle hooks (called by MemoryManager)

| Hook | When | Purpose |
|------|------|---------|
| `initialize(manager)` | Agent start | Set up connections, load state |
| `system_prompt_block()` | System-prompt assembly | Return text to inject into prompt |
| `prefetch(messages)` | Background before user turn | Pre-load relevant memories |
| `sync_turn(messages)` | After turn completes | Persist new memories from conversation |
| `on_session_end(messages)` | Session close | LLM-bound extraction; must run before session switch |
| `on_pre_compress(messages, checkpoint_api)` | Before context compaction | Durability checkpoint v1 (best-effort) or v2 (fail-closed) |
| `shutdown()` | Agent shutdown | Drain background threads |

**[VERIFIED]** — `MemoryProvider` ABC; `MemoryManager.__init__` and hooks.

### Prefetch and tool-surface plumbing

- External prefetch runs in a background thread (`ctx_bound` for contextvar
  propagation) with an **8-second timeout**. A timeout does not crash the
  agent — it returns empty context.
- Provider tool schemas are merged into the agent's tool list via
  `inject_memory_provider_tools`.
- The manager tracks `RecallStatus` (label, count, 🧠 glyph) for the
  recall indicator.

**[VERIFIED]** — `_EXTERNAL_PREFETCH_TIMEOUT_S = 8.0`;
`RecallStatus`; `is_trivial_prompt` gate (yes/ok/thanks/greetings skip
prefetch to save a round-trip).

---

## 2.9 Shipped provider plugins

Hermes ships memory provider plugins for eight external backends under
`plugins/memory/`:

| Plugin | Underlying storage |
|--------|--------------------|
| Hindsight | PostgreSQL + pgvector (embedded pg0) |
| Holographic | SQLite + FTS5 + optional NumPy HRR |
| Mem0 | Mem0 cloud (Qdrant) or self-hosted |
| OpenViking | OpenViking sidecar HTTP (port 1933) |
| Honcho | Honcho cloud or self-hosted PostgreSQL |
| Supermemory | Supermemory cloud |
| RetainDB | Undocumented (community) |
| ByteRover | Undocumented (community) |

**[OBSERVED]** — names confirmed from source imports; only the first four
(hindsight/holographic/mem0/openviking) have been researched in depth in
this report.

---

## 2.10 Assessment for OpenCode: facts vs inferences

### Transferrable concepts (**[VERIFIED]** facts)

1. **Bounded curated memory** with char budgets and §-delimited entries is
   simple, predictable, and agent-editable. Maps cleanly to a local file
   or SQLite table.
2. **Frozen system-prompt snapshot** prevents prefix-cache thrash — valuable
   when OpenCode sessions are long and the LLM cost of cache misses is high.
3. **Threat scanning at both write and load** is a model for preventing
   memory poisoning, especially relevant if memory ever enters the system
   prompt.
4. **Consolidation failure cap** prevents the agent from consuming the turn
   budget on failing memory operations — a practical guard for any tool.
5. **Drift detection** (re-read → round-trip check) guards against external
   editors or concurrent sessions corrupting the store.
6. **`<memory-context>` fencing** provides a clean semantic boundary that
   models can be taught to respect.

### What does NOT transfer directly

7. Hermes hooks are agent-internal Python calls; OpenCode uses HTTP/SSE
   events and TypeScript plugins. The plugin interface does not copy.
   **[INFERRED]** The architectural pattern (one manager, single external
   provider, prefetch background thread) is transferable; the code is not.

8. The two-file MEMORY.md/USER.md model is a good Tier 1 (in-context)
   starting point for OpenCode, but the full Hermes lifecycle depends on
   agent-loop hooks (on_session_end, on_pre_compress) that are Hermes
   internals. **[INFERRED]** OpenCode's `experimental.session.compacting`
   hook is the nearest equivalent but may break on upgrades; the adapter
   can observe session boundaries via SSE events instead.

9. The 2200/1375 char budgets are tuned for Hermes's default model context
   windows. OpenCode models vary; a budget system must be configurable.
   **[INFERRED]**