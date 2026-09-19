# §5 — Letta (letta-ai, formerly MemGPT)

> Evidence: official repo `letta-ai/letta` (Python backend) and
> `letta-ai/letta-code` (TypeScript harness), official docs
> (docs.letta.com), memory-architecture skill docs, sleep-time paper
> `arXiv:2504.13171`. Subagent deep-dive notes in
> `/tmp/opencode/subagent-notes/letta.md`.

---

## 5.1 Identity

| Attribute | Value | Label |
|---|---|---|
| Origin | MemGPT → renamed Letta | **[VERIFIED]** |
| License | Apache-2.0 | **[VERIFIED]** |
| Backend | Python 3.9+ (SQLAlchemy 2.0 async, Pydantic v2) | **[VERIFIED]** |
| Harness | `letta-code` TypeScript (Bun) | **[VERIFIED]** |
| Stars | letta ~24.8k; letta-code ~3.4k | **[VERIFIED]** |
| Key research | Sleep-time compute (arXiv:2504.13171) | **[VERIFIED]** |
| Local default | SQLite at `~/.letta/letta.db` | **[VERIFIED]** |

## 5.2 Three-tier memory architecture

**[VERIFIED]** — docs.letta.com memory-architecture.

```
LLM CONTEXT WINDOW
├── CORE MEMORY (always in-context) — [persona] [human] [custom blocks]
├── Retrieved snippets (on-demand, from archival/recall)
└── current conversation messages
        │  (retrieved via tools)
┌──────────────────┐     ┌──────────────────────────┐
│ ARCHIVAL MEMORY  │     │ RECALL MEMORY            │
│ (vector store)   │     │ (message DB)             │
│ unlimited scale  │     │ full conversation history│
└──────────────────┘     └──────────────────────────┘
```

- **Tier 1 — Core memory**: always visible; labeled blocks with
  `label`, `description`, `value`, `limit` (2000–5000 chars typical,
  ≤15 blocks), `metadata`, `read_only`.
- **Tier 2 — Archival memory**: unlimited, vector search, **explicitly**
  inserted by the agent via `archival_memory_insert`, retrieved via
  `archival_memory_search` (supports tags, pagination, temporal
  filtering).
- **Tier 3 — Recall memory**: every message ever (user/assistant/tool/
  system), agent-immutable, searched via `conversation_search`
  (semantic + time range).

**[INFERRED]** Mirrors the Atkinson–Shiffrin human-memory model
(sensory → short-term → long-term): core = working memory, archival =
semantic LTM, recall = episodic LTM.

## 5.3 Core memory blocks

- Blocks are **prepended to the system prompt** ("memory blocks are simply
  prepended to the agent's prompt in an XML-like format"). Standard render:
  `<block label="...">value</block>`; also line-numbered and git-enabled
  (MemFS) modes. **[VERIFIED]**
- **Agent self-edits** via tools:
  - `memory_insert` / `core_memory_append` — append
  - `memory_replace` / `core_memory_replace` — find/replace
  - `memory_rethink` — full block overwrite
- Limits enforced server-side; soft guidance: keep total core memory under
  80% of the context window. **[VERIFIED]**

## 5.4 Compaction (context management)

**[VERIFIED]** — `letta/services/summarizer/`.

- Reactive: token counting before each LLM call (provider-specific
  counters incl. Anthropic/tiktoken/Gemini + bytes/4 fallback); on
  `ContextWindowExceededError` → summarizer runs.
- Modes: `sliding_window` (summarize old, keep recent N%),
  `all` (whole history → one block), `self_compact_all`,
  `self_compact_sliding_window`.
- Summary inserted as system message at index 1; a lighter model is used
  for summarization; `sliding_window_percentage` default 0.5.
- File-access limits (`max_files_open`, per-file char window) prevent file
  content from eating the context.

**[INFERRED]** OpenCode already supports compaction (its `session.next.*`
compact event family was mapped in Phase 4); a memory engine should hook
the same boundary rather than implement its own compaction.

## 5.5 Sleep-time compute (memory consolidation)

**[VERIFIED]** — arXiv:2504.13171 (paper) and official blog.

- Agents "think" during idle periods — offline reasoning over context →
  better "learned context". Reported ~5× reduction in test-time compute
  on Stateful GSM-Symbolic / Stateful AIME.
- Implementation: **two agents share the same memory blocks** — primary
  (talks to user; no memory-edit tools) + background sleep-time agent
  (has memory-edit tools; runs every N steps, default 5; writes blocks;
  primary sees updates next turn).
- Benefits: consolidation off the critical path (zero user-facing latency).
- Risks: bad consolidation gets baked into shared blocks and compounds;
  every sleep-time run costs real tokens.

**[INFERRED]** Transferable as a *periodic background consolidation pass*
over a session log, not necessarily a second LLM agent. Keep it
off-the-critical-path and bounded (e.g., only at session end or on
idle).

## 5.6 Persistence

- SQLite default; PostgreSQL + pgvector for production; Redis (cache),
  GCS/cloud (MemFS cloud), ClickHouse (traces), Turbopuffer optional.
- `block` + `block_history` tables for core memory; `archival_passages`
  with embeddings; `messages` JSON; 42+ tables via SQLAlchemy 2.0.
- **MemFS** (letta-code): git-backed local filesystem for memory blocks
  (`~/.letta/memfs/`), every change committed via git — version history,
  diffing, revert, sync via remote git. **[VERIFIED]**

**[INFERRED]** Git-backed memory is a strong local-first pattern: no
server, full audit history, sync = git push/pull. Cheap to replicate.

## 5.7 Integration model

**[VERIFIED]** — client-server by design.

- `letta server` → REST API at `http://localhost:8283` + WebSocket at
  `/ws`; OpenAI-compatible endpoint via `--openai-api` flag.
- SDKs: Python (`letta-client`), TypeScript (`@letta-ai/letta-client`).
- **Any external frontend can drive it without modifying the LLM backend** —
  configure providers per agent (OpenAI/Anthropic/local).
- Integration boundary: *"The server owns agent execution, tool
  preparation, turn queueing, and event streaming. Your application owns
  product state."*

## 5.8 Assessment for OpenCode — adopt / avoid

### Adopt ([VERIFIED] concepts)

1. **Block-based mutable memory** (label + description + value + limit) —
   structured, agent-navigable, retrievable without search.
2. **Memory blocks preprended to system prompt** in a fenced/XML format —
   the same idea as Hermes's `<memory-context>`; compact, cacheable.
3. **Agent-controlled edits via tools** — tools are the interface; the agent
   is an active participant, not a passive recorder.
4. **Context budgeting + proactive compaction** — token counter, 80% budget,
   lighter summarizer model.
5. **Three-tier separation** — in-context (hot) / archival (warm) / recall
   (cold) is the fundamental scalable design.
6. **Git-backed memory versioning (MemFS)** — version history + revert +
   optional git-remote sync, fully local.
7. **Sleep-time consolidation concept** — background pass off the critical
   path (scaled down: session-end or idle-triggered, not a second agent).
8. **Client-server boundary** — a memory service with a language-neutral
   API that any client can call (HTTP/MCP), not a Python library.

### Avoid / modify

9. **Full Letta stack** (Python backend + agent harness + 42-table schema
   + pgvector + optional Redis/GCS/ClickHouse) — heavy for a TUI-side
   memory engine. Extract the *memory layer concept*, not the platform.
   **[INFERRED]**
10. **Coupling to its agent loop** — memory tools should be callable by any
    LLM-aware system, not only Letta's harness. **[INFERRED]**
11. **Mandatory vector DB** — SQLite + FTS5 covers Tier 1/2; vector search
    is an opt-in extension. **[INFERRED]**