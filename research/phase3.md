# Phase 3 — hardened standalone frontend

## What was changed

* **Backend boundary** (`src/backend/`): `Backend` trait grew a streaming
  core (`StreamEvent::{Chunk, Push, UpdateTool, ShellLine, WorkLabel}`,
  `push_stream`/`poll_stream`/`stream_active`), lifecycle ops (`new_session`,
  `cancel`, `agent_status`), stable `Session.id`, and `StatusInfo.status`.
  `MockBackend` gained per-session stream queues, deterministic word-chunked
  answers, ten `/demo a–j` scenarios, and `/new`; `cancel` drops queues and
  working indicators. No Warp/OpenCode names in the interface.
* **Streaming architecture**: polled FIFO drained by the frontend, not
  callbacks. `TuiRuntime` runs no executor (verified in
  `warpui_core/src/runtime/mod.rs`: `run_until` never drives spawned
  futures), so an earlier spawn-timer pump was replaced with a
  repaint-driven pump: the statusline element polls one event per session per
  layout (flex lays fixed children out first, so rows and footer observe one
  consistent post-poll state), and the transcript schedules ~90ms repaints
  while queued. Draw loops may present up to 3× per frame, so several events
  can land per tick; order stays deterministic.
* **Live-element pattern** (documented consequence of cached view trees):
  repaints reuse the cached tree and only re-run layout+paint, so
  time-sensitive reads moved into element `layout`: transcript rows (already),
  new `StatuslineElement` (status/hints/exit), stream polling. View re-render
  stays reserved for structural changes (keystrokes, tab sync).
* **Prompt**: multiline model (`lines`/`row`/`col`, cross-line
  navigation/join, horizontal scrolling per line, vertical growth),
  `ctrl-j` + `shift-enter` newline, multiline/sanitized paste, up/down
  fall-through to scroll at buffer edges. Blocker routing reads the backend
  live per keypress (stream-pushed gates no longer need a re-render first).
* **Tabs**: Warp's real `TuiTabBarView` hosted as an App child view
  (`add_tui_view` + `subscribe_to_view` + `TuiChildView`, Warp's own
  parent/child pattern); `ctrl-p/n` resolve adjacency through the strip
  itself; `ctrl-o` new session; invalid seeds fall back to empty + warn.
* **Scroll model fix**: the `usize::MAX` bottom-pin sentinel made relative
  scrolling from pinned state a no-op (found via action logging). Replaced
  with explicit `scroll_pinned: bool` + layout-reported totals.
* **Wheel sign**: corrected to `-delta.1` against the runtime's
  `ScrollUp → (0, 1)` convention, matching Up-arrow direction.
* **Transcript hardening**: control-byte sanitizing at the block boundary,
  mid-word wrap for long tokens, CJK width tests, all block shapes covered.
* **Provenance**: `warp-tui/` still frozen; promoted widgets document their
  allows (tab paging/adornment secondaries, exit-timer contract remnants).

## UI components completed

Tabs (real strip), transcript + all 12 block shapes, bordered slash menu,
multiline prompt with cursor, live statusline (model/cwd/branch/context +
working dot + hint precedence: exit-armed > transient > attach > default),
`?` shortcuts overlay, permission/question gates, shell/diff/plan/error/
spinner states, transient hints with enforced deadlines, double-ctrl-c
(clear → cancel → arm → exit).

## Tests

* `cargo test`: 34 passing — prompt edits/multiline/sanitize, event-routing
  through a real `App` + `TuiEventContext` (chars/ctrl/menu/blocker/wheel/
  paste/shift-enter), wrap (wide graphemes, long tokens), markdown/ANSI,
  all block shapes, backend streams/scenarios/gates/cancel/sessions, tab-bar
  config validation.
* `scripts/pty_probe.py`: 18 live checks over a direct pty with an exact ANSI
  grid parser (tmux proved unusable here: servers reaped; `send-keys` can't
  synthesize wheel; `-H` takes one byte per flag). Covers startup, scroll
  (pgup/wheel both directions), paste, ctrl-j, progressive streaming,
  gates, cancel, ctrl-o, help, exit/restoration, and 80×24 / 120×40 / 60×20 /
  40×15 resizes. All pass.
* `cargo fmt --check` clean, `cargo clippy --all-targets` zero warnings.

## Live verification performed

Everything above ran against the real binary under a pty. Implemented but
NOT live-verified: mouse clicks/hover (position-dependent; Warp-tested
primitives), shift-enter without Kitty enhancement (degrades to submit),
stream-created tab sync (strip syncs on the next action).

## Known limitations

* Single binary crate (workspace split deferred to adapter phase).
* `/demo j` sessions sync the tab strip on the next action, not instantly.
* Narrow (<~50 col) statuslines truncate the right side (Warp-identical
  truncate policy).
* Theme is a documented dark approximation (exact port = Phase 8).
* Markdown-lite (`**`, `` ` ``, links, `#`, `-`); no tables yet.
* No selection/clipboard-copy, no vim mode, no completion engine.

## Remaining Warp coupling

* `warpui_core` git dependency (MIT, pinned rev) + its element/view/runtime
  vocabulary — intentional and permanent.
* 7 AGPL snapshots under `src/tui/widgets/` (+2 test-only files' worth of
  `#[allow(dead_code)]` on snapshot-owned secondary APIs).
* Transitive AGPL crates via `warpui_core` (`warp_errors`, `warp_util`,
  `markdown_parser`) — crate stays AGPL-3.0-only (see root `NOTICE.md`).
* Zero imports from `warp` app, `ai`, settings, auth, networking, persistence.

## Licensing/provenance notes

* No new Warp files copied in Phase 3; snapshot edits limited to documented
  `#[allow(dead_code)]` + the pre-existing test-strip/provenance header.
* `research/licensing.md` analysis unchanged; re-verify before distribution.
