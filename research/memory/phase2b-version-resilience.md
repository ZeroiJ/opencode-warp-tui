# Memory Research Phase 2B — Version Resilience Design

> Status: 🔬 RESEARCH ONLY — Phase 2B design output. **Nothing here is
> implemented and nothing `experimental.*` is a hard dependency.** How the
> memory engine survives OpenCode evolution: capability detection over
> version pinning, a degradation matrix, and the rule that memory
> degradation never breaks a session.
>
> Evidence labels: **[VERIFIED]** live probe of OpenCode 2.0.8 ·
> **[DOCUMENTED]** official docs/specs/source · **[INFERRED]** analysis ·
> **[RECOMMENDATION]** Phase 2B decision → Phase 5 review.
>
> Companion: `phase2b-injection.md` (boundary, fallbacks, failure),
> `phase2b-storage.md`, `phase2b-decision-report.md` (roadmap).

---

## 1. Principle

```text
Memory is optional. Feature detection over hardcoded version assumptions.
Degradation keeps the store working and the session normal.
```

The memory engine has **one external dependency: a single HTTP surface**
(the session instruction-entries endpoint). Everything else is local. That
surface is unstable by nature:

- Live 2.0.8 serves it at `/api/experimental/session/{id}/instructions/
  entries[/{key}]` **[VERIFIED]** while the official docs document the
  non-experimental `/api/session/{id}/…` alias **[DOCUMENTED]** — which
  returns **404 on 2.0.8** **[VERIFIED]**.
- The value limit is **262,144 bytes** today **[VERIFIED]** — a number, not
  a contract.
- The V2 Context Epoch model shows injection semantics (baseline vs
  chronological update) are version-sensitive **[DOCUMENTED]**.

So the adapter must **probe, not assume**, on every connect.

---

## 2. Capability detection (decision)

Per connect (or per server *version* change detected via
`/api/info` — cheap to cache per version string **[RECOMMENDATION]**):

1. **Probe**: create one throwaway session; `PUT` a tiny entry
   (`{"value":{"text":"probe"}}` under a `owt.probe` key); `GET` it;
   `DELETE` it; delete the session. All 2xx ⇒
   `injection.surface = entries`. Any failure ⇒ surface absent.
2. **Path preference list** (ordered, first success wins):
   `experimental` variant (known-good on 2.0.8), then documented
   non-experimental alias. Each is exercised by the probe, so the probe
   itself resolves the path. **[RECOMMENDATION]**
3. **Cache** the result keyed by server version from `/api/info`
   (version string → surface + last round-trip time). Re-probe when the
   version string changes, the probe is stale past a TTL (e.g. 1 h), or a
   live injection attempt fails unexpectedly.
4. Probe failures are **silent to the model** and surfaced to the user
   once (a warning line in the adapter's UI log), then injections are
   disabled for the session.

The probe is self-cleaning (deletes its session) and tiny;
**[RECOMMENDATION]** it runs opportunistically at session creation, never
synchronously on the user's first prompt path (injection §3 sequencing
allows this: the probe precedes the memory PUT).

---

## 3. Degradation matrix

| OpenCode condition | Detection | Memory behavior | Session behavior |
|---|---|---|---|
| Entries endpoint removed (404 on all paths) | Probe fail | Injection **disabled**; store commands still work | Normal |
| Endpoint moved to a new path | Probe fail on known paths; new path unknown | Injection disabled until a future release adds the path — **or** probe of the documented alias succeeds and is adopted automatically | Normal |
| Entry key pattern tightened (`owt.memory` rejected, 400/422) | Live PUT fails | Injection disabled for the session with a warning | Normal |
| Value limit lowered below our safety limit | Live PUT 413 despite budget | Builder's encoded-byte variant should hold; if still 413 → drop block, warn | Normal |
| Response shape / contract drift (unexpected JSON, 5xx on valid calls) | Probe or live PUT parse/validation failure | Disable injection, log ids/timestamps only (security §8) | Normal |
| Auth changes (401/403) | Probe fail | Disable injection; report auth issue once | Normal (server auth is Phase 4's domain anyway) |
| Server version unknown/very new | Version header absent → run probe regardless (probe is version-agnostic) | Works or disables cleanly per above | Normal |
| OpenCode downgrade (2.0.8 → N-1) | Version string change → cache invalidated → fresh probe | Same decision tree | Normal |
| OpenCode unavailable (server down) | Adapter connection fails | **Memory store commands remain functional** (they are local file ops). Injected-memory path simply can't run | Existing Phase 4 behavior (session commands fail); store ops are independent |
| Session APIs change shape | Phase 4 mapper already isolates this; memory only needs `session_id` (unchanged concept) | Unaffected | Normal |
| Compaction/context-epoch behavior changes | Not consumed by V1 (frozen block travels at session start; mid-session entry updates never used) | Unaffected | Normal |

### 3.1 What V1 deliberately does NOT do

- No version-string whitelisting ("this works with 2.0.8 only").
- No feature gating on things memory doesn't use (events, plugins,
  prompts) — only the one surface is probed.
- No mid-session re-injection "recovery" — if the PUT failed at session
  start, the session simply runs without memory (frozen semantics, D18).

---

## 4. Failure model (hard rule, reaffirmed)

From P2A §16 and injection §9, verbatim in effect:

```text
Memory failure ≠ OpenCode failure.
```

Every row of §3: a memory failure changes *memory's* state
(unavailable/disabled) and produces at most one user-visible warning;
the OpenCode session runs untouched. Store failures (permissions,
corruption, size guard) take the same shape at a lower layer
(storage §8, security §7).

---

## 5. Version-resilient element checklist (what survives change)

| Element | Why it survives |
|---|---|
| Memory format (`v:1`, JSONL, tombstones) | OpenCode-independent; `v` guards future schema drift (storage §4) |
| Key/id addressing | Content-format property of the store, not OpenCode |
| Budget arithmetic | Bounded by the **measured** limit with margin (retrieval §3) — re-measured at probe time, not hardcoded in docs |
| Fenced block format | Plain text; degrades gracefully even if rendered as raw JSON (injection §2) |
| Command surface (`/memory …`) | Adapter-side interception; unaffected by OpenCode commands (injection §8) |
| Probe + degradation tree | Version-agnostic by construction |

---

## 6. Version-gate implementation guidance (Phase 5)

Minimal shape (`phase2b-decision-report.md` §5 roadmap):

```text
struct InjectionSurface { path_candidates: Vec<Path>, max_bytes: u64, available: bool }
- file: injection::probe(&client, &session) -> Surface   // self-cleaning
- file: injection::put_block(&client, &session, block)   // uses Surface
- config knob: "memory.injection" = auto | off           // off = never inject
- resolvers: one probe per server-version change; cache in memory
```

No global-config changes to OpenCode are ever made (AGENTS.md: live
server *reads* only in this phase). The adapter's version probe touches
**its own** sessions only.

---

## 7. Relationship to Phase 4 detection

Phase 4 already performs connect-time capability detection in the adapter
(project status: adapter foundation complete). Memory reuses that
detection seam and adds **its own** narrow probe for the single surface it
needs. No duplication of general capabilities; no shared mutable state
with the TUI.

---

## 8. Version-resilience test strategy (Phase 5)

| Area | Tests |
|---|---|
| Probe | Against sandboxed servers with: experimental path only (2.0.8), documented alias only, both, neither (stub server) → correct surface |
| Cache | Version change invalidates; TTL expiry re-probes; live-failure triggers re-probe |
| Degradation | Each matrix row has a fixture/stub test: injection disabled ⇒ session messages untouched, store commands still succeed |
| 413 race | Stub server with a low limit → builder truncates or drops block, no crash |
| Offline | Server absent: store CRUD works; injection path no-ops |
| Data isolation | Probe sessions deleted (no leaks); store never touched by probes |

---

## ARCHITECTURE STATUS

```text
Version resilience: resolved. Implementation NOT authorized.
```

Capability detection, the path-preference rule, the degradation matrix,
and the store-keeps-working invariant are the Phase 5 contract.