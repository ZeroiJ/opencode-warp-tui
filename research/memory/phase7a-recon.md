# Phase 7A — Research & Architecture Reconnaissance

> Status: 🔬 RESEARCH ONLY. **No production code modified, no behavior
> changed, no dependencies added, `~/warp` untouched.** Read-only source
> inspection + read-only live OpenAPI fetch from the local OpenCode 2.0.8
> server. No sessions created, no prompts sent, no model calls.
>
> Evidence labels: **[VERIFIED]** tested/probed · **[SOURCE]** OWT or
> OpenCode source read · **[INFERRED]** reasoned conclusion ·
> **[DOCUMENTED]** project or official docs state it · **[OPEN]** not
> established.
>
> Companion: `phase7a-decision-log.md` (7A-R1…). Authoritative roadmap:
> `phases.md` + `README.md` (agree verbatim); architecture baseline:
> Phase 2B package, Phase 5 implementation, Phase 6A recon, `PHASE6B_COMPLETE.md`,
> `phase6c-verification.md`.

---

## 1. Status

Phase 7A in progress → this document + decision log are the deliverables.
Phase 7B is not started and must not start without explicit authorization.

## 2. Research scope

Determine what Phase 7 should build from project evidence (not
assumptions): roadmap wording, Phase 4 leftovers, current adapter/TUI
gaps, and the live 2.0.8 API surface. Memory internals (Phase 5/6) are
explicitly out of scope except as frozen foundations.

## 3. Current architecture (as built)

```text
TUI (src/tui: session/transcript/prompt/menus/statusline)
  │  Backend trait (src/backend/mod.rs — snapshots + poll_stream pump)
  ├── MockBackend (scripted parity backend)
  └── OpenCodeBackend (src/backend/opencode/)
        ├── client.rs (blocking ureq, 15 s timeout, tolerant Value parsing)
        ├── memory_inject.rs (Phase 6B: probe + PUT owt.memory at create)
        ├── MemoryApi + memory/ (Phase 5: frozen)
        └── mapper.rs + events.rs (SSE → StreamEvent → Block)
```

- Backend trait already exposes `resolve_permission`, `answer_question`,
  `cancel`, `new_session`, `commands`, `status`, `blocker` **[SOURCE]**.
- TUI already calls all three gate actions (`SessionAction::AnswerQuestion`
  → digits resolve live Permission/Question blockers; `NewSession`;
  `Submit`) **[SOURCE]** `src/tui/session.rs:623-806`.
- Slash menu exists, fed by `backend.commands()` (`menus.rs`,
  `session.rs:415-427`) **[SOURCE]**.
- MockBackend implements the full trait incl. gates/cancel/new_session —
  parity testbed exists **[SOURCE]**.

## 4. Existing roadmap evidence (authoritative)

`phases.md:337` and `README.md:88` agree word-for-word **[DOCUMENTED]**:

> **Phase 7 — Full Agent Interaction.** Complete whatever remains from the
> Phase 4 report: **questions, permissions, tools, shell, files, diffs,
> cancellation, errors, session history, reconnect behavior.**

Consequences:

- Phase 7 is **agent interaction**, not memory UI. Memory UX is explicitly
  Phase 9 ("TUI Memory UX … UX not finalized") **[DOCUMENTED]**.
- Memory intelligence is Phase 8; hardening is Phase 10. Neither leaks
  into Phase 7.
- Terminology preserved: "Full Agent Interaction".

## 5. Phase 7 candidate scope (from evidence)

### 5.1 Phase 4 leftovers, reconciled with what exists today

| Phase 4 / roadmap item | Current state | Gap for Phase 7 |
|---|---|---|
| Questions (answer delivery) | `answer_question` renders a Notice: "answer delivery lands in Phase 5" (stale comment); gate tracked in `_pending_questions` (unused — underscore prefix) **[SOURCE]** | **Core item.** Delivery mechanism is [OPEN] (§6) |
| Permissions | `reply_permission` wired (once/reject); `always` reserved; saved-permissions API unused **[SOURCE]** | Polish: `always` allow-list UI? saved permissions? candidate |
| Tools / shell / files / diffs | Mapped to Tool/Shell/Edits blocks already (mapper event table) **[SOURCE]** | Verify completeness vs 2.0.8 vocabulary; likely small |
| Cancellation | `cancel` → `POST …/interrupt` wired **[SOURCE]** | Reconnect/busy reconciliation after cancel |
| Errors | Error blocks on tool/step failure + prompt rejection **[SOURCE]** | Audit coverage; likely small |
| Session history | Hydration on select; "old sessions return `[]`" limitation from Phase 4; no pagination (`?limit=200` fixed) **[DOCUMENTED + SOURCE]** | Hydration proof + pagination candidate |
| Reconnect behavior | SSE worker + resubscribe; "torture + busy reconciliation after SSE gaps" recommended, never done **[DOCUMENTED]** | Robustness item |
| Context-% | `status()` maps model/cwd/branch; context_pct source unverified here | `GET …/context` exists on 2.0.8 — candidate |
| Model/agent switching | `GET /api/model`, `/api/agent` unused by adapter | `POST …/model`, `POST …/agent` exist — candidate (Phase 4 item 5) |
| Slash-command execution | Commands listed only; `POST …/command` (execute callback) unused | Candidate (extends existing menu seam) |
| Compact / diff / revert / fork | Routes exist on 2.0.8, unused | Candidates, need product decision |
| Tab auto-reveal on append | Phase 4 limitation, still present per report | Small TUI item |

### 5.2 Live 2.0.8 surface relevant to Phase 7 (read-only fetch, this phase)

Spec: OpenAPI 3.1.0, 113 paths, fetched from the live server
(`/openapi.json` → `/tmp`, outside the repo) **[VERIFIED]**.
Interaction routes the adapter does **not** use yet:

```text
POST …/command            execute a slash-command callback (name+text)
POST …/compact            compaction request (steers at step boundary)
GET  …/diff               structured per-file diffs of a turn
POST …/model              switch model for subsequent turns
POST …/agent              (agentID) — switch agent
POST …/form               / GET …/form / POST …/form/{id}/reply (frm_)
POST …/shell              shell execution route
POST …/environment        / POST …/move / POST …/fork / POST …/view
GET  …/message/{id}       single-message fetch (pagination seam)
PATCH …/session/{id}      session update; GET /api/session/active
DELETE …/session/{id}     (adapter never deletes sessions)
GET  …/permission/*       saved permissions; POST …/permission (session)
```

Notably **absent**: any `/question/` route — `question.v2.asked` carries
`que_` ids with no documented REST answer target (only `frm_` form-reply
exists). The Phase 4/5 comment "no question-reply route" is therefore
**still true on 2.0.8's documented surface** (version note updated:
1.18.31 → 2.0.8) **[SOURCE]**.

## 6. Current UX/capability gap (memory question, §9)

What the user can do with memory today (all via `/memory`, in-band
transcript replies) **[SOURCE]** `command.rs` + adapter `memory_reply`:

- remember / update / forget / pin+unpin / list (scope/kind/pinned
  filters) / show (+`--all` history) / help. No search, no export
  (grep confirms neither exists).
- Injection visibility: none in-TUI (by design — frozen pre-prompt entry;
  statusline shows model/ctx/cwd/branch only).

Gap assessment: the memory command surface is complete per its V1
contract; the remaining memory UX (browsing, visibility of injected
state) is **Phase 9 by roadmap**. Phase 7 must not absorb it
(7A-R1).

## 7. TUI architecture findings (extension seams)

- `SessionView::handle_action` is the single gesture→backend funnel;
  adding a Phase 7 interaction = new `SessionAction` variant + backend
  trait method (already the pattern for gates) **[SOURCE]**.
- `blocker()` (Permission|Question on trailing block) drives gate UX;
  digits double as option selectors only when a blocker is live
  **[SOURCE]**.
- `SlashMenu::for_buffer(&backend.commands(), …)` + `selected_command()`
  is the palette seam; menu selection path exists at `session.rs:679`
  (currently submits text) — command *execution* via API would attach
  here **[SOURCE]**.
- Transcript renders all `Block` variants incl. Permission/Question/
  Edits/Shell/Error/Notice — no new block types obviously required for
  Phase 7 unless diffs need richer rendering (`FileDiff` exists)
  **[SOURCE]**.
- Statusline consumes `StatusInfo{model, context_pct, cwd, branch}` —
  context_pct improvement needs no TUI change **[SOURCE]**.

## 8. Command architecture findings

- `/memory` + `/mem` intercepted in `OpenCodeBackend::submit` **before any
  network call**; `\/memory` escape; malformed → in-band `Block::Error`,
  never forwarded **[SOURCE]** (Phase 5/6A recon).
- Replies are plain strings formatted by `command::{written,forget,pin,
  list,show}_reply` into Assistant/Error blocks — transcript-visible,
  synchronous, backend-local **[SOURCE]**.
- Server-side slash commands are list-only today; execution would reuse
  the same menu seam (§7), not the `/memory` prefix path. No conflict.

## 9. External research

None required. Warp patterns (menus, tab strip, block list) are already
promoted in-tree under the established licensing split; OpenCode's REST
surface was read from the primary source (live spec). Per §12 rule, no
external feature (Hermes/Hindsight/Mem0/Letta/OpenViking UX) enters the
contract — nothing in the roadmap asks for it. Warp repo untouched
(verified clean before/after).

## 10. Options (unranked, per §14)

**O-A — Gates-only (minimal).** Wire `answer_question` delivery
(mechanism TBD §11-Q1) + permission `always`/saved-permissions polish.
Files: `client.rs` (+1 route call), `mod.rs` (`answer_question`,
drop `_pending_questions` underscore), mapper test fixtures, mock parity,
PTY gate scenario. Compat: full (trait unchanged). Risk: low — except the
delivery mechanism is [OPEN].

**O-B — A + history/robustness.** Add message pagination
(`GET …/message?limit/offset` — verify param names live),
hydration proof, SSE-gap torture + busy reconciliation, cancel-edge
behavior. Files: `client.rs`, `mod.rs` (hydrate paths), harness tests.
Risk: medium (timing-sensitive tests).

**O-C — B + session operations.** Compact, diff view, revert, fork,
model/agent switching, slash-command execution — each behind explicit
product decisions (keybinding vs `/`-command vs menu). Files: client
wrappers + small TUI affordances reusing §7 seams. Risk: medium-high —
scope creep is the failure mode; each item needs its own acceptance line.

**O-D — C + statusline/context accuracy.** context_pct via `GET …/context`,
model label after switches. Small, testable.

Each option satisfies: no Phase 5/6 semantic change, no new deps
(ureq+serde_json cover all routes), mock parity extendable. The decision
required is **scope breadth** (7A-R4), not architecture.

## 11. Decisions required (→ decision log)

- 7A-R1: Phase 7 excludes memory UX (Phase 9 owns it).
- 7A-R2: Q&A delivery mechanism stays [OPEN] pending live 7B verification
  (candidates: answer-as-prompt-text vs undocumented route).
- 7A-R3: Frozen list (§12) — Phase 5/6 untouched.
- 7A-R4: Scope breadth among O-A…O-D left to the Phase 7B authorizer
  (recommendation: authorize A+B, hold C item-by-item).
- 7A-R5: No new deps; no migration; Backends compat (mock implements every
  new trait method or the method stays adapter-local).

## 12. Frozen assumptions

```text
Phase 5 memory schema / MemoryStore / MemoryApi semantics
memory ordering (D24) / budget algorithm / injection format
owt.memory key / session snapshot + resume semantics
capability probe + failure-isolation behavior
Backend trait shape (extend only additively, with mock parity)
TUI block vocabulary (extend only if diffs demand it — deferred to 7B)
```

Removal from this list requires demonstrating a genuine defect (none
found — §30 clear).

## 13. Proposed Phase 7B contract

### APIs (adapter-local; trait additive-only)
- `Client::answer_question(session, request_id, option)` — exact shape
  fixed by 7B live verification ([OPEN] mechanism).
- `Client::{get_messages_paged, post_command, post_compact, get_diff,
  post_model, post_agent}` — only the subset authorized under §10 scope.
- `MemoryApi`/injection: **no changes**.

### Data flow
```text
gate event (SSE) → mapper → Block::Permission|Question → blocker()
  → digit action → Backend::{resolve_permission|answer_question}
  → Client route call → worker event confirms → transcript updates
history: set_active → hydrate (paged) → blocks
ops (compact/diff/…): key or /-command → Client call → Notice/blocks
```

### State
No new persistent state. Pending-gate ids already tracked in adapter
`State` (promote `_pending_questions` to used). No store migration:
**Migration: NONE**.

### Configuration
None anticipated. If model/agent switching needs defaults, reuse existing
config conventions (env override pattern in `config.rs`) — 7B decides.

### Commands
No new `/memory` subcommands. Slash-menu execution (if authorized) sends
selected server command via `POST …/command`, not via prompt text.

### UI
Reuse only: trailing-block gates, digit selection, slash menu,
statusline, Notice/Error blocks. New keybindings only where a gate/ops
item cannot ride existing ones (7B documents each).

### Backend
OpenCodeBackend only; every trait addition mirrored in MockBackend.

### Testing
Unit (mapper fixtures, client stub-server per new route), PTY harness
gate scenarios (both backends), live read-only + controlled live gate
delivery (7B plan), regression (fmt/clippy/full suite + 18/18 PTY green).

### Failure handling
Existing rule extends: interaction failure → in-band Error/Notice, warn
once, session continues. Question delivery failure must visibly state
the answer was NOT delivered (no pretending — extends the Phase 4 honesty
rule).

### Security
No new secret surfaces (routes carry ids/options, never memory content).
Permission `always` requires explicit user confirmation per use (no silent
persistent grants).

## 14. Test plan

| Layer | Content |
|---|---|
| Unit | mapper fixtures for new events; client tests per route vs stub server; command/menu pure logic |
| Integration | mock parity for every trait addition; hydrate/pagination fixtures |
| TUI/PTY | gate render + digit resolve on both backends; new affordances in harness |
| Live | read-only route checks; controlled gate-delivery verification (trigger a real permission/question — 7B's heaviest probe, planned not executed here) |
| Regression | fmt, clippy `-D warnings`, full `cargo test`, 18/18 PTY both backends, warp clean |
| Manual | end-to-end gate flows against live server + model |

## 15. Non-goals (explicit)

Memory UI/browsing (Phase 9) · memory search/export · automatic
extraction (Phase 8) · SQLite/embeddings/retrieval changes · TUI redesign
· new persistence models · new dependencies · OpenCode-side changes ·
any Phase 5/6 semantic change.

## 16. Open questions

1. **[OPEN]** Question delivery mechanism (answer-as-prompt vs hidden
   route) — live 7B verification required.
2. **[OPEN]** Scope breadth (O-A…O-D) — authorizer decision.
3. **[OPEN]** `GET …/message` pagination parameter names — 7B reads live
   spec/edge-probes (spec fetch showed `limit=200` in use; offset/cursor
   form unconfirmed).
4. **[OPEN]** Whether `always` permission surfacing is wanted at all.
5. **[OPEN]** context_pct computation source (current mapper value
   provenance not traced in this phase — 10-minute 7B task).

## 17. Acceptance criteria (for 7B sign-off)

- Every authorized scope item has a live-verified route or a documented
  negative result with fallback behavior.
- Gates resolve end-to-end against the live server (or the negative is
  explicit and user-visible).
- Full suite green + 18/18 PTY both backends + warp clean + no new deps.
- Frozen list (§12) violated nowhere (diff review).
- No memory-UX feature merged (Phase 9 guard).

---

## Appendix — source classification index

Roadmap wording [DOCUMENTED] (phases.md, README) · adapter/TUI behavior
[SOURCE] (file:line refs inline) · 2.0.8 routes [SOURCE] (live spec
fetch) · question-route absence [SOURCE] (spec-wide grep) · delivery
mechanism [OPEN] · scope recommendation [INFERRED] · nothing in this
document is [VERIFIED] by execution (no code ran beyond read-only GETs
of `/api/info` and `/openapi.json`).
