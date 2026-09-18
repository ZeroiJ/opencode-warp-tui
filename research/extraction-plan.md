# Extraction plan

## 1. What can be extracted directly

* **MIT element library** (`warpui_core::elements::tui` + `TuiView`/`TuiPresenter`/
  driver): depend on `warpui_core` with the `tui` feature (license intact) or
  vendor it preserving MIT notices. Elements (`TuiFlex/Text/Stack/Container/
  ViewportedList/Selectable/…`), buffer/geometry/event types need no changes.
* **7 copied snapshots** in `warp-tui/src/` (pure presentation/state, verified
  zero `warp::`/`ai::` imports): `tui_column_layout`, `tab_bar`, `link`,
  `transient_hint`, `exit_confirmation`, `input_hints`, `osc_notifications`.
  They still target `warpui_core` APIs and will need import remapping once the
  host context is chosen.
* **Ideas/patterns** (no code needed): transcript-over-block-list with
  viewport source + registries; one-mode input arbitration
  (`TuiInputSuggestionsMode`); blocking-input arbitration (`BlockingInputSource`);
  per-tool bespoke views with a generic fallback; permission prompt and
  ask-question cards over a shared option selector; semantic style builder.

## 2. What must be adapted

* `TuiUiBuilder` → reimplement over a local theme/palette (drop
  `Appearance`/`WarpTheme`); every view consumes it, so this is the first seam.
* Transcript/agent/tool views → rebind `warp::tui_export` model handles to
  backend traits (block list, action/history models, subagent controller).
* `tui_markdown` → swap `markdown_parser` (AGPL) for an independent renderer
  or vendor it with license intact; point palette at the new builder.
* `TuiInputView`/editor stack → abstract `CodeEditorModel`/`CoreEditorModel`
  behind a text-model trait; keep prompt policy.
* Driver/`TuiView`/`AppContext` entity system → shim or reimplement the
  invalidation-driven loop (`spawn_tui_driver` shape) for the new host.
* Menus → rebuild each against OpenCode data sources, keeping the inline-menu
  routing + snapshot pattern.

## 3. What must be rewritten

* `session::run` / `warp::run_tui` bootstrap, `RootTuiView` auth flow,
  `TuiSessions` lifecycle → around the OpenCode client/service model
  (discovery/start/health per the OpenCode skill: `opencode service …`,
  `opencode api …`).
* Statusline/usage/billing/zero-state content → OpenCode context, permissions,
  git, and account realities.
* Keybinding registration → re-derive once views exist (keep conventions).

## 4. What should NOT be copied

* The entire `crates/warp_tui` directory (120 backend-coupled files).
* `app/src/terminal/cli_agent*` internals (reference only; OSC/plugin patterns
  inform the adapter, they are not the TUI).
* Warp settings/auth/telemetry/persistence/networking stacks.
* Test harness files (`*_tests.rs`, benches, `tui_test_support`) until a local
  harness exists.

## 5. OpenCode counterparts for Warp dependencies (to pin in Phase 4–5)

| Warp dependency | OpenCode replacement (verify against current docs/SDK at the time) |
| --- | --- |
| `TerminalModel` + block list | Session/message list from OpenCode API (`opencode api get …`, client/SDK streaming) |
| `BlocklistAIAction/HistoryModel`, `AIAgentAction*` | Tool-call/message stream incl. states, diffs, command output |
| `CLISubagentController` | Background-task / session-fork endpoints |
| `AskUserQuestion*` + `Option*` snapshots | Question/permission prompts + answers API |
| Permission accept/reject/guidance | Permissions API |
| Slash-command mixer, models menu, MCP menus | Commands/agents list, models list, MCP servers endpoints |
| `warp::editor::CodeEditorModel` | Local text-model implementation owned by this project |
| `Appearance`/`WarpTheme` | Local palette + terminal-theme probing (cf. `terminal_background.rs` idea) |
| `warp::run_tui` bootstrap | `opencode service start/status` + managed background service |
| `keybindings` | `cli.json` keybinds model (TUI-side; separate from `opencode.json(c)`) |

## 6. Proposed architecture for the standalone project

```text
warp-tui/                      # presentation only; no backend imports
  elements/  (warpui_core tui dep or vendored MIT subset)
  views/     (transcript, tool views, input, menus — over backend traits)
  styles/    (local ThemeBuilder seam replacing TuiUiBuilder)
backend-kit/                   # traits: SessionSource, ToolStream, Permissions,
                               Questions, Commands, Models, Mcp, Files, Status
  opencode-adapter/            # the ONLY crate importing the OpenCode SDK/API
app/                           # bootstrap: service discovery, driver loop, keymap
```

Rules: views depend on `backend-kit` traits, never on the adapter; the adapter
depends on traits + OpenCode; `app` wires them. This preserves backend
replaceability and keeps rendering decoupled from Phase 5 onward.
