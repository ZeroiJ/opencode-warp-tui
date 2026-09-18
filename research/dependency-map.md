# Dependency map

Conventions: **Type** ∈ {UI, state, rendering, terminal, agent protocol,
backend, filesystem, networking, application-specific}.
**Can Reuse?** = Yes (as-is) / Adapt (needs interface change) / No (rewrite or
leave behind). Every classification was verified by reading imports.

## A. TUI views → Warp backend (via `warp::tui_export`, `app/src/tui_export.rs`)

`warp::tui_export` (351 lines, `#[cfg(feature="tui")]`) re-exports ~200 types.
120 of 185 `warp_tui` files import it. It is the dominant coupling surface.

| Warp TUI Component | Warp Dependency | Type | Can Reuse? | Required Adaptation |
| --- | --- | --- | --- | --- |
| `TuiTranscriptView` | `TerminalModel`, `BlocklistAIAction/HistoryModel`, `ModelEventDispatcher`, `AIBlockModelImpl`, block-list types | state, terminal, agent protocol | Adapt | Replace model handles with backend-trait snapshots; keep viewport/registry structure |
| `TuiTerminalSessionView` (5787 lines, ~40 export types) | `BlocklistAIController/InputModel`, `CLISubagentController`, `AgentConversationsModel`, `TuiMcpManager`, `Sessions`, `TerminalSurface`, slash-command mixer, telemetry | state, backend, agent protocol, application-specific | No (this phase) | Too coupled to copy; reimplement surface, reuse its *structure* (transcript+input+menus+blockers) |
| `TuiAIBlock` + sections | `AIAgentAction/Text/Todo`, `AIBlockModel*`, `BlocklistAI*`, billing/team types | agent protocol, backend | Adapt | Define OpenCode-native action/message types; keep section-render decomposition |
| Tool views (shell/file-edits/permission/plan/ask-question/generic) | `AIAgentAction*`, `AskUserQuestion*`, `Option*` snapshots, `DiffSessionType/FileDiff` | agent protocol, state | Adapt | Rebind to OpenCode tool-call/permission/question protocol; keep interaction patterns |
| `TuiCLISubagentView` | `CLISubagentController/Target`, `LongRunningCommandControlState`, `ShellCommandExecutor` | agent protocol, terminal | Adapt | Replace with OpenCode background-process handle |
| `TuiInputView` + editor stack | `warp::editor::CodeEditorModel`, `BlocklistAIInputModel`, `warp_editor::CoreEditorModel` | state, terminal | Adapt | Needs an editor-model abstraction; prompt policy (shell `!`, vim, menus) is reusable logic |
| Menus (slash/conversation/model/team/skills/mcp/history/completion) | `SlashCommandMixer`, `AgentConversationsModel`, `LLMPreferences`, `warp_completer`, `warp_search_core` | backend, application-specific | No | Rebuild against OpenCode (commands, models, MCP); keep `TuiInputSuggestionsMode` arbitration idea |
| `TuiSessions` / `session::run` / `root_view` | `warp::run_tui` full-app bootstrap, `TuiLoginModel`, `TerminalManagerTrait`, `PersistenceWriter` | backend, networking, filesystem | No | Rewrite bootstrap around OpenCode client/service; login flow is Warp-specific |
| Statusline / usage / zero-state | `ConversationUsageTotals`, `UserWorkspaces`, `GitRepoModels`, `ChangelogModel`, `SkillManager`, `warp::settings::*` | backend, networking, application-specific | No | Redesign for OpenCode context/billing/git; keep layout ideas |
| `TuiUiBuilder` | `tui_export::Appearance`, `warp_core` theme (`WarpTheme`) | rendering, application-specific | Adapt | **Key seam**: reimplement builder over a local theme/palette; all views consume it |
| `tui_markdown` + table | `markdown_parser` crate (AGPL workspace) + `TuiUiBuilder` | rendering | Adapt | Swap parser (or vendor with license intact) + local palette |
| `tool_call_labels`, `warping_indicator` | `ai::agent::*`, `tui_export::format_credits` | agent protocol, application-specific | Adapt | Trivial logic once backend types exist; not copied (has `ai::`/`warp::` imports) |
| `read_only_menu`, `ui.rs`, `shortcuts/status_menu` | `TuiUiBuilder` (transitively Warp theme) | UI, rendering | Adapt | Copyable only after builder is decoupled |
| `keybindings.rs` | all major views (hub) | state | No | Re-derive after views exist; keep `tui:` naming + `TUI_BINDING_GROUP` convention |
| `app/src/terminal/cli_agent*` (reference only, not TUI) | OSC 777 parsing, per-agent plugin managers | agent protocol, terminal | Adapt | `CLIAgent` enum already has an `OpenCode` variant; plugin-manager pattern is the model for a future adapter |

## B. Rendering infrastructure (MIT — reusable)

| Component | Origin | Type | Can Reuse? | Required Adaptation |
| --- | --- | --- | --- | --- |
| `TuiElement` trait + `TuiFlex/Text/Stack/Container/ConstrainedBox/Clipped/Scrollable/ViewportedList/Selectable/ChildView/EventHandler/Hoverable/collapsible/Animated/ShimmeringText/SizeConstraintSwitch` | `warpui_core::elements::tui` (MIT, `tui = ["dep:ratatui"]`, ratatui 0.30) | rendering | Yes | None for elements; they are backend-agnostic |
| `TuiBuffer/Cell/TuiStyle/Color/Modifier`, geometry (`TuiSize/Rect/Constraint/Point`), events (`TuiEvent`, dispatch results), scene/z-order | same | rendering | Yes | None |
| `TuiView` trait (`warpui_core/src/core/view/tui.rs`), `TuiPresenter` (`presenter/tui.rs`), `spawn_tui_driver`/`TuiRuntime` (`runtime/mod.rs:715`) | `warpui_core` (MIT) | rendering, state | Adapt | Deeply coupled to Warp `App/AppContext/Entity/ViewHandle`/keymap: needs entity-system shim or reimplementation of the driver loop |
| `AppContext/Entity/ModelHandle/ViewHandle`, actions/keymap, `Appearance`, `FeatureFlag`, telemetry, logging | `warp_core`/`warpui` (MIT / AGPL respectively — verify per crate) | state | Adapt | Either depend on these crates with licenses intact or define a minimal host-context trait |

## C. Copied in Phase 1 (closed pure set — zero `warp::`/`ai::`/`warp_*` imports)

| File | Imports | Why safe to copy |
| --- | --- | --- |
| `warp-tui/src/tui_column_layout.rs` | `warpui_core` only | Pure layout math |
| `warp-tui/src/tab_bar.rs` | std + `unicode-segmentation` + `warpui_core` | Generic tab-bar component |
| `warp-tui/src/link.rs` | `warpui_core` only | Tiny presentation helper |
| `warp-tui/src/transient_hint.rs` | std + `warpui_core` | Self-contained timed-hint state |
| `warp-tui/src/exit_confirmation.rs` | std + `instant` | Zero Warp deps at all |
| `warp-tui/src/input_hints.rs` | none | Pure string helpers |
| `warp-tui/src/osc_notifications.rs` | none (empty module) | Placeholder preserved for structure |

## D. Deliberately NOT copied (with reason)

* All 120 files importing `warp::tui_export` (transcript, session, agent/tool
  views, input, menus, bootstrap) — backend-coupled; need adapter types first.
* `tui_builder.rs` — theme-coupled; must be reimplemented, not copied.
* Files transitively coupled via `TuiUiBuilder`: `tui_markdown*`,
  `read_only_menu.rs`, `ui.rs`, `warping_indicator.rs`, `tool_call_labels.rs`,
  `terminal_session_view/{shortcuts,status_menu}.rs`.
* `keybindings.rs` — hub importing every view; re-derive later.
* Re-export shims (`input/mod.rs`, `handoff/mod.rs`, `attachment_bar/mod.rs`) —
  their targets are coupled; copying shims alone is misleading.
* All `*_tests.rs`, benches, `src/bin/*` — need the Warp test harness /
  full app; noted as future reference for render-to-lines patterns.
