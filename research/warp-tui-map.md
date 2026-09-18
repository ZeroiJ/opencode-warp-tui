# Warp TUI map

Reference: Warp @ `1bf1c6a2b` (2026-09-17). All paths relative to repo root.
Every entry below was read (headers + imports + key items), not guessed.

## 1. Relevant directories

| Directory | Role |
| --------- | ---- |
| `crates/warp_tui/` | The headless TUI front-end ("Warp Agent CLI"). 185 `.rs` files, ~3.8 MB. Builds per-channel binaries (`src/bin/oss|local|dev|preview|stable.rs`). AGPL-3.0-only. |
| `crates/warp_tui/src/` | All TUI views, state models, menus, input, keybindings, session bootstrap. |
| `crates/warp_tui/src/terminal_session_view/` | Sub-modules of the main session surface (`state.rs`, `statusline.rs`, `completions.rs`, `input_detection.rs`, `shortcuts.rs`, `status_menu.rs`, `todo_menu.rs`, `usage_menu.rs`+`model.rs`). |
| `crates/warp_tui/src/input/` | Editor-backed prompt (`mod.rs`, `view.rs`, `vim.rs`). |
| `crates/warp_tui/src/attachment_bar/`, `handoff/` | Attachment bar and local-to-cloud handoff sub-modules. |
| `crates/warpui_core/src/elements/tui/` | MIT-licensed cell-grid element library (~460 KB): `TuiElement` trait + ~20 concrete elements. Reusable presentation vocabulary. |
| `crates/warpui_core/src/runtime/` (`mod.rs:715`) | `spawn_tui_driver` — production invalidation-driven TUI driver. Warp-entity-coupled. |
| `crates/warpui_core/src/presenter/tui.rs` | `TuiPresenter` / `TuiFrame` — layout + paint of a `TuiView` tree. |
| `crates/warpui_core/src/core/view/tui.rs` | `TuiView` / `AnyTuiView` trait definitions. |
| `app/src/terminal/` | GUI+shared terminal model; `cli_agent.rs` (656 lines), `cli_agent_sessions/` (OSC 777 event listener, per-agent plugin managers incl. `opencode.rs`). |
| `app/src/tui_export.rs` (351 lines) | `#[cfg(feature = "tui")] pub mod tui_export` — re-exports ~200 Warp-internal types as the TUI's single backend import surface. |
| `app/src/lib.rs:940` | `pub fn run_tui(api_key, mount: TuiMountFn)` → `run_internal(LaunchMode::Tui{..})`. Full headless Warp app bootstrap (settings mode `Tui`, `.tui` secret namespace, windowless `AppBuilder`). |

## 2. Relevant files — purpose, structs, rendering, events

### Session bootstrap & shell

| File (lines) | Purpose | Key structs / events | Warp coupling |
| --- | --- | --- | --- |
| `session.rs` (419) | Binary entry `run()`; parses CLI (`--resume`, `--api-key`, …), calls `warp::run_tui`, mounts root view, starts driver + `TuiSessions` | `TuiArgs`, `TuiCommand::DumpSettingsSchema`, `init()`, `ensure_terminal_session()` | `warp::run_tui`, `warp::TuiLoginModel/Phase`, `spawn_tui_driver`, `warp::settings::*` |
| `root_view.rs` (414) | Login-gated app shell; shows Auth or focused Terminal session | `RootTuiView` (`RootTuiState::{Auth,Terminal}`), `RootTuiAction::{ExitApp, StartDeviceLogin, …}` | `warp::TuiLoginModel/Phase`, `tui_export::{ServerId, TeamUpdateManager, UserWorkspaces}` |
| `session_registry.rs` | Owns live sessions, focus, lifecycle | `TuiSessions`, `TuiSession{Id,View}`, `TuiSessionsEvent::{SessionRemoved, FocusChanged}` | ~15 `tui_export` types (`TerminalManagerTrait`, `PersistenceWriter`, …) |
| `keybindings.rs` (197) | Aggregates every view's `init(app)`; TUI binding validators (`TUI_BINDING_GROUP="tui"`, `tui:` prefix) | `init(app)`, `is_tui_owned_binding()` | Imports all major views (hub — do not copy) |
| `ui.rs` (659) | Auth/welcome/restore placeholder screens | `signed_out_welcome()`, `login_waiting/failed()`, `conversation_restoring()` | `TuiUiBuilder`, zero-state animation |
| `tui_builder.rs` (634) | Semantic theme→style recipes (the TUI `UiBuilder`) | `TuiUiBuilder::from_app(app)`, `primary/muted/dim/accent/error/success…_style()` | `tui_export::Appearance`, `warp_core` theme types — **adapters must replace this** |
| `resume.rs` | Carries resume token across teardown | `TuiExitSummaryHandle` | `tui_export::ServerConversationToken` |

### Transcript / session rendering (core agent UI)

| File (lines) | Purpose | Key structs / render fns |
| --- | --- | --- |
| `transcript_view.rs` (829) | Scrollable transcript over canonical block-list order; owns agent/CLI-subagent/handoff/terminal block registries | `TuiTranscriptView`, `TuiTranscriptViewEvent::{SelectionStarted/Ended, BlockingStateChanged, PermissionReplacementGuidanceSubmitted}`, `TRANSCRIPT_BLOCK_SPACING` |
| `terminal_session_view.rs` (5787) | Authenticated session surface: transcript + input + attachment bar + inline menus + blocking-input arbitration | `TuiTerminalSessionView`, `TuiTerminalSessionAction` (~30 variants), `TuiTerminalSessionEvent::{ExecuteCommand, InterruptPty, WriteAgentInput, …}`, `BlockingInputSource::{LongRunningCommand, AskQuestion, Permission, Orchestration, Handoff}` |
| `terminal_session_view/state.rs` | Session state machine snapshot resolved from weak entity refs | `TuiTerminalSessionStateModel`, `TuiTerminalSessionState::{Block, AltScreen}`, `TuiFirstZeroStateState` |
| `terminal_session_view/statusline.rs` | Footer statusline (context usage, clock, git, hints) | `format_context_window_usage`, `render_statusline_datetime`, `format_todo_progress` |
| `terminal_session_view/{completions,input_detection}.rs` | Shell completion coordination; Agent-vs-shell input classification | `CompletionRequestState`, `InputDetectionState/Decision` |
| `terminal_session_view/{shortcuts,status_menu,todo_menu,usage_menu}.rs` | Stateless read-only menu projections (`?`, `/status`, TODOs, `/usage`) | `menu()`, `active_todo_menu()`, `TuiStatusInfo`, `TuiUsageSnapshot`, `TuiUsageCreditBar` |
| `terminal_content_element.rs` | Transparent PTY wrapper: publishes content size, forwards PTY input/mouse | `TuiTerminalContentElement`, `MouseReportPolicy`; impls `TuiElement` |
| `terminal_block.rs` | Paints terminal cells for one transcript block | `TerminalBlockElement`, `block_content_rows()`, `should_render_terminal_block()` |
| `tui_block_list_viewport_source.rs` | Adapts terminal block-list order to `TuiViewportedList` | `TuiBlockListViewportSource`, `AgentBlockRegistry`, `CLISubagentBlockRegistry`, `OVERHANG_ROWS=20` |

### Agent / tool-call rendering

| File (lines) | Purpose | Key structs / events |
| --- | --- | --- |
| `agent_block.rs` (2015) | One exchange: user input + agent response; section extraction/composition | `TuiAIBlock`, `TuiAIBlockEvent/Action`, `CollapsibleSectionStates` |
| `agent_block_sections.rs` (311) | Pure per-section render fns | `render_input_section`, `render_thinking_section`, `render_todo_list_section`, `render_summarization_section`, `render_fallback_tool_call_section` |
| `agent_message.rs` | Orchestration participant messages | `render_agent_message`, `conversation_status_glyph` |
| `tui_cli_subagent_view.rs` | Agent monitoring a long-running command | `TuiCLISubagentView(Event)`, `terminal_use_status_text` |
| `tui_generic_tool_call_view.rs` | Permission-capable fallback for bespoke-less tool calls | `TuiGenericToolCallView(Event)` |
| `tui_shell_command_view.rs` (—) | `RequestCommandOutput` disclosure + embedded terminal renderer | `TuiShellCommandView(Action/Event)`, `ShellCommandViewState` |
| `tui_file_edits_view.rs` | `RequestFileEdits` diff wrapper over editor element | `TuiFileEditsView(Event/Action)`, `FILE_EDITS_PERMISSION_ACTIVE` |
| `tui_code_block_view.rs` | Read-only code block over char-cell `CodeEditorModel` | `TuiCodeBlockView(Event)`, `TuiCodeBlockPayload`, `MAX_HIGHLIGHT_BYTES/LINES` |
| `tui_permission_prompt.rs` (385) | Reusable Yes/No/Other permission prompt + body editor | `TuiPermissionPrompt(Action/Event)`, `PERMISSION_PROMPT_ACTIVE/EDITABLE` |
| `tui_plan_view.rs` | Inline Markdown for Create/EditDocuments calls | `TuiPlanView(Event/Action)` |
| `tui_ask_question_view.rs` | Interactive `AskUserQuestion` card | `TuiAskQuestionView(Action/Event)`, `ASK_QUESTION_ACTIVE/MULTISELECT_ACTIVE` |
| `tui_review_comments.rs`, `tui_plan_view`, `orchestration_block*/`, `orchestration_tab_bar.rs` | Review comments, plan docs, orchestration pills/tabs | `TuiOrchestrationBlock`, `TuiOrchestrationModel`, tab-bar paging |
| `tool_call_labels.rs` (754) | Status labels for tool calls | `ToolCallDisplayState`, `tool_call_display_state/label()`, `styled_tool_call_label_spans()` |
| `tui_markdown.rs` (462) + `table.rs` (313) | `FormattedText` → element tree | `TuiMarkdownPalette`, `TuiMarkdownBlockHooks`, `render_formatted_text/table` |
| `tui_column_layout.rs` (126) | Two-column width allocation | `TuiTwoColumnConstraints/Layout`, `tui_two_column_layout()` — **pure, copied** |

### Input / prompt

| File (lines) | Purpose | Key structs / events |
| --- | --- | --- |
| `input/view.rs` (1553) | `TuiInputView`: prompt backed by char-cell `CodeEditorModel`; shell mode (`!`), vim, inline menus, completion | `TuiInputView(Event::{Submitted, Pasted, AcceptedSlashCommand/Conversation/Model/…, VimModeChanged, …})`, `TuiInputAction`, `TuiCompletionInputSnapshot` |
| `input/vim.rs` | `VimHandler` for the prompt (`!`-prefix shell mode; most motions no-op) | `VimHandler` impl |
| `editor_element.rs` (924) | Char-cell editor element (TUI `RichTextElement`); paints/interacts, no row computation | `TuiEditorElement`, `TuiEditorAction::{InsertChar, PasteText, Selection*, Scroll}`, `TuiEditorStyles` |
| `editor_view.rs` | Generic focusable text field (no prompt policy) | `TuiEditorView(Event/Action)` |
| `editor_interaction.rs` | Shared editing commands, clipboard, viewport scroll | `TuiEditorCommand` (~20 variants), `TuiEditorBehavior/State`, `apply_editor_action/paste()` |
| `input_suggestions_mode.rs` | One-mode-owns-input arbitration | `TuiInputSuggestionsMode::{Closed, SlashCommands, ApiKeys, ConversationMenu, ModelSelector, …}`, `TuiInputSuggestionsModeModel` |
| `inline_menu.rs` | Active-menu routing + cell presentation (`MAX_INLINE_MENU_ROWS=10`) | `TuiInlineMenuHandle`, `TuiInlineMenuSnapshot/Row/Accepted` |
| `{completion_menu, slash_commands, conversation_menu, model_menu, team_menu, skills_menu, mcp_menu, mcp_install_flow, prompt_and_command_history_menu}.rs` | Per-menu query state + snapshots | `*Model` / `*State` / `*Event` each; all funnel through `TuiInputSuggestionsModeModel` |
| `attachment_bar/{mod,model,view}.rs` | Image/file attachment bar | `TuiAttachmentBar(Event)`, `TuiAttachmentModel` |
| `tab_bar.rs` (1000) | Generic responsive retained tab bar — **pure (std+unicode+warpui_core), copied** | `TuiTabBarView/Config/Event/Action`, `TuiTabBarPagingState` |
| `option_selector.rs` | Generic option list used by permission/ask-question/statusline-config | `TuiOptionSelector(Action/Event)` |
| `zero_state{,_animation}.rs` | Pre-first-interaction screen | `TuiZeroStateView`, `ZeroStateAnimationElement` |

### Small pure helpers (copied as reference)

`link.rs` (43, `TuiLink`), `transient_hint.rs` (123, `TransientHint/Tone`),
`exit_confirmation.rs` (68, double-ctrl-C window), `input_hints.rs` (7, hint
strings), `osc_notifications.rs` (0, empty placeholder module).

## 3. Relationships

```text
session::run() → warp::run_tui() → init() → keybindings::init() (all views)
  → spawn_tui_driver() → RootTuiView{Auth|Terminal} → TuiSessions
  → TuiTerminalSessionView ─┬─ TuiTranscriptView ─┬─ TuiAIBlock ─┬─ section views
                            │                     │               ├─ TuiShellCommandView ─ TerminalBlockElement
                            │                     │               ├─ TuiFileEditsView ─ TuiEditorElement
                            │                     │               ├─ TuiPermissionPrompt ─ TuiOptionSelector
                            │                     │               └─ TuiAskQuestionView ─ TuiOptionSelector
                            │                     ├─ TuiCLISubagentView
                            │                     └─ TerminalBlockElement (via viewport source)
                            ├─ TuiInputView ─ TuiEditorElement + inline menus
                            │                 (arbitrated by TuiInputSuggestionsModeModel)
                            ├─ TuiAttachmentBar ─ TuiAttachmentModel
                            └─ statusline / read-only menus / zero state
```

All data flows through `warp::tui_export` model handles
(`TerminalModel`, `BlocklistAI*Model`, `CLISubagentController`, …); all
rendering flows through MIT `warpui_core::elements::tui` elements with
semantic styles from `TuiUiBuilder` (which itself needs the Warp theme —
the key seam for a future backend swap).
