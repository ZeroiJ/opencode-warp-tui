# §8 — OpenCode memory projects (community survey)

> Evidence: subagent ecosystem survey (~25 projects) with GitHub/npm checks;
> `anomalyco/opencode` issues; official ecosystem listings. Notes in
> `/tmp/opencode/subagent-notes/opencode-projects.md`.

---

## 8.1 OpenCode itself has no built-in memory

- **[VERIFIED]** `anomalyco/opencode` issue #24030: "Currently, OpenCode
  does not have a built-in memory system."
- **[VERIFIED]** Issue #20322: feature request for native auto-memory.
- **[VERIFIED]** Live 2.0.8 OpenAPI (242 schemas) exposes session/message/
  instruction-entry/cost APIs but no memory object; no built-in memory.
- **Consequence**: every memory solution is a plugin, sidecar, or
  adapter-side feature. **[INFERRED]**

## 8.2 Landscape (selected, with labels)

| Project | Type | Storage | OpenCode integration | Label |
|---|---|---|---|---|
| `tickernelz/opencode-mem` | npm `opencode-mem` v2.25 (976 ⭐ npm, 97 dependents) | **Turso/libSQL** native vector (`F32_BLOB`, `vector_top_k`); web UI :4747 | Plugin; 3-layer search (index → timeline → fetch); auto-capture | **[VERIFIED]** de facto standard |
| `supermemoryai/opencode-supermemory` | Official ecosystem listing | Supermemory cloud or self-hosted | Plugin; context injection on session start; compaction at 80% | **[VERIFIED]** |
| `joshuadavidthomas/opencode-agent-memory` | Official ecosystem "featured" | Local markdown blocks + journal; all-MiniLM-L6-v2 local embeddings | Plugin; `experimental.chat.system.transform`; requires OpenCode ≥ 1.0.115 | **[VERIFIED]** |
| `plastic-labs/opencode-honcho` | Plugin + Python core (AGPL-3.0 core, MIT plugin) | Honcho backend (PostgreSQL; cloud or self-hosted) | Plugin; "dreamer"/"reconciler" background reasoning | **[VERIFIED]** |
| `cnicolov/opencode-plugin-simple-memory` | Fork of `shuans/opencode-memory` | `~/.config/opencode/memory/` logfmt files | Plugin; 5 tools (`memory_remember/recall/update/forget/list`) | **[VERIFIED]** |
| `@mem0/opencode-plugin` | Official mem0 plugin | Mem0 cloud (Qdrant) or self-hosted; local via `ZeR020/opencode-mem0` | Pure TS plugin; 9 tools + 9 skills; hooks on session start/every prompt/compaction | **[VERIFIED]** |
| `@vectorize-io/opencode-hindsight` | Official hindsight plugin | Hindsight server (Postgres/pg0) :8888 | Plugin; retain/recall/reflect; session.idle auto-retain | **[VERIFIED]** |
| `@openviking/opencode-plugin` | Official OpenViking plugin | OpenViking sidecar :1933 | MCP (stdio); 15 tools; auto-recall/auto-capture | **[VERIFIED]** |
| `JosXa/opencode-recall` | Explicit-by-design recall | SQLite sidecar index; **reads OpenCode's own session DB** | `agent.recall` config; no auto-injection | **[VERIFIED]** |
| `jackmazac/opencode-engram` | Research-leaning | SQLite + FTS5 + embedding blobs; RRF merge | Plugin; streaming cosine retrieval; `experimental.chat.system.transform` | **[VERIFIED]** |
| `sdwolf4103/opencode-working-memory` | Zero-config automatic | Local files | **Piggybacks on `experimental.session.compacting`**; native TUI `/memory` menu | **[VERIFIED]** |
| `mathew-cf/opencode-memory` | Git-tracked notes hybrid | `~/opencode-memory/` git repo + `rag` CLI embeddings | Compaction-time retrospective; search nudge at 8 tool calls | **[VERIFIED]** |
| `lwfeng-ch/opencode-memory` | Pipeline-centered | Local, configurable | Auto-extraction; two-stage recall (rule filter + LLM rerank); Dream consolidation (4-phase) | **[VERIFIED]** |
| `devcxl/opencode-memory` | OpenClaw-style 9-file | Local markdown + Cloudflare Workers (optional) | Plugin; `/memory-init`; per-file locking | **[VERIFIED]** |
| `csuwl/opencode-memory-plugin` | OpenClaw-style 9-file + vectors | `~/.opencode/memory/` SQLite + sqlite-vec | Plugin; session auto-sync; 8 tools incl. `vector_memory_search` | **[VERIFIED]** |
| ~10 more | MCP servers, claude-mem ports, forks | SQLite/markdown | Mostly low-activity | **[OBSERVED]** |

## 8.3 Patterns distilled

### Hook mechanisms used by the ecosystem

| Mechanism | Used by | Stability |
|---|---|---|
| `experimental.chat.system.transform` | agent-memory, engram, lkonga, opencode-mem | **[VERIFIED]** not in current official docs — at risk |
| `experimental.session.compacting` | working-memory, mathew-cf | **[VERIFIED]** in current docs — still experimental prefix |
| `agent.recall` (native config) | opencode-recall | **[VERIFIED]** docs presence; newer feature |
| Tool registration via plugin SDK | almost all | **[VERIFIED]** stable plugin API |
| MCP server (stdio/http) | dony102, ngrabbs, mem0-standalone, hindsight, openviking | **[VERIFIED]** standard; client must support MCP |

**[INFERRED]** The ecosystem leans on **experimental/at-risk hooks** for
injection. Anything we ship should prefer (a) plugin API (stable), (b)
HTTP surface we verify per version, or (c) adapter-side observation
(TUI-own) — never an undocumented hook.

### Storage patterns

- SQLite variants dominate: libSQL/Turso-native-vector, sqlite-vec,
  sql.js, better-sqlite3, FTS5 + embedding blobs.
- Markdown-file memory (logfmt, YAML frontmatter, §-delimited) is the
  simplest tier; commonly git-tracked.

### Retrieval patterns

- 2-stage (rule filter → LLM rerank); 3-layer (index → timeline →
  details); multi-signal fusion (semantic + BM25 + entity — mem0);
  RRF merge (engram); plain keyword + semantic.

## 8.4 Red flags ([VERIFIED] / [OBSERVED])

1. **Extremely low star counts** (0–3 ⭐) — limited community validation.
2. **Fork proliferation** — many are forks of `shuans/opencode-memory`
   with minimal changes.
3. **Version pinning** — agent-memory requires ≥ 1.0.115; several pin
   `@opencode-ai/plugin@1.2.x`.
4. **Experimental-hook damage** — plugins targeting
   `experimental.chat.system.transform` may break with OpenCode upgrades
   (and that hook is absent from current docs).
5. **Cloud deps** — Supermemory/Mem0/Honcho need accounts (all have
   self-hosted options).
6. **Unmaintained/early-release** — several 0-commit or explicitly
   unstable projects.
7. **License ambiguity** in some projects — check before reuse.

## 8.5 Reusable ideas ([VERIFIED] from listed projects)

1. **3-layer progressive retrieval** (compact index → timeline → full
   details) for token efficiency — opencode-mem, lucasliet.
2. **Compaction-time extraction** (piggyback on the compacting hook OR
   observe compact events adapter-side) — working-memory, mathew-cf.
3. **RRF merge** of multiple signals — engram.
4. **Git-tracked memory directories** — mathew-cf.
5. **Reading OpenCode's own session SQLite/DB instead of duplicating
   storage** — opencode-recall. **[INFERRED]** relevant for Episode-tier
   (recall) memory: our adapter already sees session IDs; a read-only
   reader could reuse OpenCode's history without schema changes.
6. **Explicit-by-design recall** (no auto-injection) — opencode-recall.
7. **Memory pressure detection + Dream consolidation** — lwfeng-ch.
8. **Governance/audit logs** — lwfeng-ch, shuans.
9. **Cross-agent memory via MCP** — rajarshighoshal, Honcho, mem0.
10. **Native TUI memory menu** (`/memory`) — working-memory; maps to
    Phase 9 UX plans.
11. **Scope management** (user/project/global) — mem0 `/mem0-scope`,
    lkonga three-tier scopes.
12. **Session plan storage with TTL** — chriswritescode-dev.

## 8.6 What this means for the architecture decision

**[INFERRED]**
1. Building **our own adapter-side memory engine** (rather than adopting
   one plugin) avoids the experimental-hook fragility that plagues most
   ecosystem projects, and keeps the "isolate OpenCode behind the
   adapter" rule intact.
2. The **de facto community choices** (Turso/libSQL, FTS5+vector, markdown)
   confirm SQLite-class storage as the pragmatic local-first baseline.
3. The single most reliable integration surface used everywhere is the
   **plugin API + tool registration** — but our project does not need a
   plugin if the adapter itself can capture events and inject context via
   supported HTTP/attachment boundaries (verified in §8.1 sources and
   the live 2.0.8 probes).
4. Whatever we do, **avoid depending on `experimental.*` hooks** as a
   hard requirement; treat them as accelerators behind a feature flag.