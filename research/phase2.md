# Phase 2 — standalone foundation

## Decisions

1. **Render on real Warp infrastructure.** The crate depends on MIT
   `warpui_core` (git, pinned to Phase-1 revision `1bf1c6a2b`, `tui` feature)
   instead of vendoring or reimplementing the element library. All views render
   genuine `TuiElement` trees (`TuiFlex/Text/Container/EventHandler/Animated`,
   …) through `TuiRuntime`, exactly like `warpui_core`'s own `tui_file_viewer`
   example. Rationale: maximal visual fidelity now, and future Warp view ports
   compile against the APIs they were written for. Consequence: the crate is
   AGPL-3.0-only overall (AGPL snapshots + AGPL transitive crates
   `warp_errors`/`warp_util`/`markdown_parser`) — see root `NOTICE.md`.
2. **Compatibility layer, not rewrite.** The 7 Phase-1 snapshots were promoted
   to `src/tui/widgets/` with byte-identical content except: stripped
   `#[cfg(test)] mod tests;` (Warp harness files not extracted) + one
   provenance header line. Pristine copies stay frozen under `warp-tui/`.
   `tab_bar` compiles but is not yet hosted (needs App child-view wiring —
   Phase-3 task, `#[allow(dead_code)]` marked); the other six are genuinely
   used (column layout, exit confirmation, transient hints, input hints, link,
   osc placeholder).
3. **Backend seam first.** Views depend only on the local `Backend` trait
   (`src/backend/mod.rs`, object-safe, `Rc<RefCell<dyn Backend>>`); all data
   in Phase 2 comes from `MockBackend` (scripted Warp-like sessions).
4. **Theme seam first.** `src/theme.rs` re-implements Warp's `TuiUiBuilder`
   semantic vocabulary over a documented dark-palette approximation (two brand
   literals are exact: lilac `#D2B5FF`, green `#E2FFD4`). Exact theme port +
   light scheme + background probing = Phase 8.
5. **Simplified-but-honest deviations** (full list): single-line prompt (Warp
   has a multiline editor model); markdown-lite instead of `markdown_parser`;
   permission/question gates answer via `1/2/3` on the last block instead of
   input-replacing blockers; transcript scroll via wheel/pgup/pgdn/↑/↓ (no
   `j/k` — those type); no streaming (responses land whole; spinner covers
   progress); simple text tab strip (real `TuiTabBarView` hosting later);
   wheel-scroll sign to verify live.

## Changes from upstream (promoted files)

* Removed 3-line `#[cfg(test)] #[path] mod tests;` footers (5 files).
* Added 1-line provenance header (7 files).
* Nothing else.

## New code inventory

* `src/main.rs` — `App::test` bootstrap + `TuiRuntime` loop (mirrors
  `tui_file_viewer`).
* `src/theme.rs` — semantic styles over approximated dark palette.
* `src/backend/mod.rs` — `Backend` trait + `Session/Block/ToolCall/ShellRun/
  FileDiff/PermissionRequest/Question/StatusInfo` models.
* `src/backend/mock.rs` — two scripted sessions + cycling canned turns.
* `src/tui/session.rs` — `SessionView` (header/transcript/menu/prompt/
  statusline, `SessionAction` dispatch, exit confirmation, help overlay).
* `src/tui/transcript.rs` — block→row rendering, wrap, markdown-lite,
  braille spinner over `TuiAnimated`.
* `src/tui/prompt.rs` — `PromptState` + cursor element (`>`/`!` prefixes).
* `src/tui/menus.rs`, `src/tui/statusline.rs` — slash menu, footer.
* `Cargo.toml` (binary `owt`), root `NOTICE.md`, `mise.toml` (Rust 1.92.0).

## Verification

Phase-2 exit state (2026-09-18, tmux 100×30 runs): typing, submit, `/` menu
(filter/accept), `?` help, `ctrl-t` activity, permission `1/2/3` answering,
`ctrl-p/n` tabs, `!` shell prefix, pgup scroll-to-top, braille spinner
animation (frames advanced across captures), truecolor ANSI output confirmed
via `capture-pane -e`, resize reflow, double-ctrl-c exit with terminal
restoration. Zero `cargo check`/`clippy` warnings, `cargo fmt` clean.
(`cargo test` arrived during Phase 3: 9 tests, all passing.)

Known Phase-2 gaps carried into Phase 3: no streaming (whole responses),
single-line prompt, `tab_bar.rs` compiled but unhosted, wheel-sign unverified,
no repo documentation of the keymap.
