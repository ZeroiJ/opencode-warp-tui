# Phase 7A — Decision Log

> Research only. No production change. IDs `7A-R1…`. Nothing here revises
> Phase 2B (D1–D25), Phase 5, or Phase 6.

## 7A-R1 — Phase 7 excludes memory UX

- **ID:** 7A-R1
- **Question:** Is Phase 7 a memory-UI phase?
- **Evidence:** phases.md:337 + README.md:88 define Phase 7 as "Full Agent
  Interaction … questions, permissions, tools, shell, files, diffs,
  cancellation, errors, session history, reconnect"; Phase 9 owns "TUI
  Memory UX" [DOCUMENTED].
- **Decision:** Phase 7B implements no memory browsing, visibility, or
  management UI.
- **Rationale:** Roadmap is explicit and unanimous; absorbing memory UX
  would starve the agent-interaction mandate and collide with Phase 9.
- **Impact:** Memory UX gap documented as Phase 9 backlog, not Phase 7
  scope.

## 7A-R2 — Question delivery mechanism stays OPEN

- **ID:** 7A-R2
- **Question:** How are `question.v2.asked` (`que_`) answers delivered?
- **Evidence:** No `/question/` route in the live 2.0.8 OpenAPI (113
  paths, spec-wide grep) [SOURCE]; only `frm_` form-reply exists;
  `answer_question` currently posts a Notice instead of delivering
  [SOURCE] `opencode/mod.rs:567`.
- **Decision:** Do not pick a mechanism in 7A. Phase 7B must live-verify:
  (a) answer-as-prompt-text vs (b) undocumented route, then implement the
  verified one with a visible not-delivered fallback.
- **Rationale:** Guessing wrong bakes a silent failure into gate UX.
- **Impact:** 7B's first task is a live gate-delivery probe.

### 7A-R2 — RESOLVED in Phase 7B (live probe 7B-P1)

- **Evidence:** Live probe against OpenCode 2.0.8 (2.0.8, service at
  `http://127.0.0.1:49374`, Basic auth): the `question` tool materializes
  as `form.created` SSE (`metadata.kind:"question"`, `frm_` id,
  `fields[].key` "q0", `options[].value/label`); `question.v2.asked`
  never fires live (the old listener was dead). Delivery is
  `POST /api/session/{sid}/form/{formID}/reply` with body
  `{"answer": {"q0": <value>}}` → `204 No Content`; the tool completes
  with `metadata.answers: [["value"]]`. Lifecycle:
  `execution.started → form.created → form.replied → execution.succeeded`.
  SDK `/api/session/{sid}/question/...` routes return 404. Full shapes in
  `research/memory/phase7b-question-probe.md`; capture files under
  `/tmp/owt-probe-*.jsonl`.
- **Decision:** The verified form-reply route is the delivery mechanism.
  Replies send option **values** (≠ labels); the adapter tracks
  `form_id` + `field_key` + `option_values` per gate. `question.v2.asked`
  remains a rendered gate that resolves locally with a visible notice on
  servers without the form route (no verified reply path).
- **Impact:** `answer_question` delivers via `reply_form`; wire shape
  stub-tested (`client.rs` tests) and probe-documented. No `/question/`
  SDK route used.

## 7A-R3 — Frozen list for Phase 7B

- **ID:** 7A-R3
- **Question:** What must 7B not touch?
- **Evidence:** Phase 5/6 verified stable (151 tests, 6C matrix)
  [DOCUMENTED]; no defect found in this phase.
- **Decision:** Freeze: memory schema/store/API, ordering, budget,
  injection format, `owt.memory`, snapshot/resume semantics, probe
  behavior; Backend trait additive-only with mock parity.
- **Rationale:** Phase 7 is completion of interaction, not memory
  redesign; §30 stop conditions all clear.
- **Impact:** 7B diff review checks this list line by line.

## 7A-R4 — Scope breadth deferred to authorizer (recommend A+B)

- **ID:** 7A-R4
- **Question:** How much of O-A…O-D does 7B take?
- **Evidence:** Roadmap lists ~10 areas without prioritization; adapter
  already covers tools/shell/cancel rendering; gaps cluster at gates +
  history/robustness [SOURCE + INFERRED].
- **Decision:** 7A recommends authorizing O-A (gates) + O-B
  (history/robustness), holding O-C items (compact/diff/revert/fork/
  model-agent switching/command execution) for item-by-item approval.
- **Rationale:** Gates are the only honest-to-goodness broken path
  (Notice instead of delivery); history/reconnect are named roadmap
  items; O-C risks scope creep without per-item acceptance.
- **Impact:** 7B prompt must carry the authorized subset explicitly.

## 7A-R5 — No new dependencies, no migration, mock parity rule

- **ID:** 7A-R5
- **Question:** Constraints for 7B implementation?
- **Evidence:** All candidate routes work over existing ureq/serde_json
  [SOURCE]; no new persistent state proposed (§13) [INFERRED].
- **Decision:** No new crates; Migration: NONE; every trait addition
  mirrored in MockBackend; failure → in-band Error/Notice, never fatal.
- **Rationale:** Keeps 7B a completion phase, not a platform phase.
- **Impact:** 7B sign-off includes dep-diff and mock-parity checks.
