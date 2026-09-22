//! Session surface: tab strip, transcript, inline menu, prompt, statusline.
//!
//! Structure mirrors Warp's `TuiTerminalSessionView` (transcript + input +
//! menus + blocking-input arbitration) with the Warp models replaced by the
//! local [`Backend`](crate::backend::Backend). All Warp-visual contracts kept:
//! one blank row between blocks, `>`/`!` prompt prefixes, accent selection,
//! scroll/clamp/pin behavior.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use warpui_core::elements::tui::{
    TuiChildView, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiEventHandler, TuiFlex,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiScreenPosition, TuiSize,
};
use warpui_core::{
    AppContext, Entity, TuiView, TypedActionView, UpdateView, ViewContext, ViewHandle,
};

use super::menus::{render_menu, SlashMenu};
use super::prompt::{PromptElement, PromptHistory, PromptState};
use super::statusline::StatuslineElement;
use super::transcript::{collapse_block_rows, row_element, wrap_spans, zero_state_rows, TxRow};
use super::widgets::{
    format_tui_first_column, tui_two_column_layout, ExitConfirmation, TransientHint,
    TransientHintTone, TuiLink, TuiTab, TuiTabBarConfig, TuiTabBarNavigationDirection,
    TuiTabBarView, TuiTwoColumnConstraints, TRANSIENT_HINT_DURATION,
};
use crate::backend::{Backend, Blocker};
use crate::theme::Theme;
use instant::Instant;

/// Single-line prompt edits (typed actions from the prompt element).
#[derive(Clone, Debug)]
pub enum PromptEdit {
    Insert(String),
    Newline,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    WordBack,
    WordForward,
    KillToStart,
    KillToEnd,
    KillWordBack,
}

/// Transcript scroll requests.
#[derive(Clone, Copy, Debug)]
pub enum ScrollDelta {
    Lines(i32),
    Page(i32),
}

/// Menu navigation.
#[derive(Clone, Copy, Debug)]
pub enum MenuNav {
    Prev,
    Next,
    Accept,
}

/// Actions handled by [`SessionView`]. All payloads are owned data only
/// (`Action: Send + Sync`), never live borrows.
#[derive(Clone, Debug)]
pub enum SessionAction {
    Prompt(PromptEdit),
    Submit,
    Menu(MenuNav),
    Scroll(ScrollDelta),
    /// Recall an older (`-1`) or newer (`+1`) submitted prompt, or scroll
    /// when no history applies (single-line prompt, empty ring, gate live).
    History(i32),
    ToggleCollapse,
    QuitOrCancel,
    ToggleHelp,
    DismissTop,
    LinkOpened(String),
    AnswerQuestion(usize),
    SimulateActivity,
    CycleTab(i32),
    NewSession,
}

impl SessionAction {
    pub fn prompt_edit(edit: PromptEdit) -> Self {
        SessionAction::Prompt(edit)
    }
}

/// Shared view state handed to per-frame elements. Bundled so constructors
/// stay narrow while every element observes the same live snapshots.
#[derive(Clone)]
struct ViewShared {
    backend: Rc<RefCell<dyn Backend>>,
    theme: Theme,
    link: TuiLink,
    viewport: Rc<Cell<usize>>,
    total_rows: Rc<Cell<usize>>,
    hint_deadline: Rc<Cell<Option<Instant>>>,
}
///
/// Scrollable transcript body: pre-wrapped single-row elements with exact
/// row skipping. The viewport height is observed in `layout` (the flex child
/// constraint), so bottom-pinning and clamping need no terminal queries.
///
/// Repaint scheduling lives in `render`: while a stream is queued it asks for
/// a follow-up repaint (~90ms), which keeps frames coming. The events
/// themselves are polled by the statusline element, which flex lays out first
/// (see its docs). `TuiRuntime` runs no executor, so no spawned timer futures
/// are involved anywhere in the pump.
pub struct TranscriptBody {
    backend: Rc<RefCell<dyn Backend>>,
    theme: Theme,
    link: TuiLink,
    help: bool,
    scroll: usize,
    pinned: bool,
    /// Fold every long shell/diff/thinking block to its header row.
    collapse_all: bool,
    viewport: Rc<Cell<usize>>,
    total_rows: Rc<Cell<usize>>,
    hint_deadline: Rc<Cell<Option<Instant>>>,
    rows: Vec<Box<dyn TuiElement>>,
    skip: usize,
    visible: usize,
}

impl TranscriptBody {
    fn new(
        shared: &ViewShared,
        help: bool,
        scroll: usize,
        pinned: bool,
        collapse_all: bool,
    ) -> Self {
        Self {
            backend: shared.backend.clone(),
            theme: shared.theme,
            link: shared.link.clone(),
            help,
            scroll,
            pinned,
            collapse_all,
            viewport: shared.viewport.clone(),
            total_rows: shared.total_rows.clone(),
            hint_deadline: shared.hint_deadline.clone(),
            rows: Vec::new(),
            skip: 0,
            visible: 0,
        }
    }

    fn logical_rows(&self, width: usize) -> Vec<TxRow> {
        if self.help {
            return help_rows();
        }
        let backend = self.backend.borrow();
        let summaries = backend.session_summaries();
        let Some(summary) = summaries.get(backend.active()) else {
            return Vec::new();
        };
        let blocks = backend.session_blocks(&summary.id);
        if blocks.is_empty() {
            return zero_state_rows(self.theme);
        }
        let mut rows = Vec::new();
        for block in blocks.iter() {
            // One blank row above every block (Warp's BLOCK_TOP_PADDING_ROWS).
            rows.push(TxRow::Text(Vec::new()));
            rows.extend(collapse_block_rows(self.theme, block, self.collapse_all));
        }
        // Wrap logical rows into visual single-line rows at the known width.
        let mut visual = Vec::with_capacity(rows.len());
        for row in rows {
            match row {
                TxRow::Text(spans) if spans.iter().all(|(s, _)| s.is_empty()) => {
                    visual.push(TxRow::Text(Vec::new()));
                }
                TxRow::Text(spans) => {
                    for line in wrap_spans(&spans, width) {
                        visual.push(TxRow::Text(line));
                    }
                }
                other => visual.push(other),
            }
        }
        visual
    }
}

impl TuiElement for TranscriptBody {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let width = constraint.max.width;
        let viewport = constraint.max.height as usize;
        self.viewport.set(viewport.max(1));
        let logical = self.logical_rows(width as usize);
        let total = logical.len();
        self.total_rows.set(total);
        let max_skip = total.saturating_sub(viewport.max(1));
        self.skip = if self.pinned {
            max_skip
        } else {
            self.scroll.min(max_skip)
        };
        self.visible = viewport.max(1);
        self.rows = logical
            .into_iter()
            .skip(self.skip)
            .take(self.visible)
            .map(|row| {
                let mut element = row_element(self.theme, &self.link, row);
                element.layout(TuiConstraint::tight(TuiSize::new(width, 1)), ctx, app);
                element
            })
            .collect();
        TuiSize::new(width, constraint.max.height)
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        for (i, row) in self.rows.iter_mut().enumerate() {
            row.render(origin.offset(0, i as i32), surface, ctx);
        }
        // Keep the pump alive while streams are queued, and wake up to retire
        // an armed transient hint on time. Both stop rescheduling on their own.
        if self.backend.borrow().stream_active() {
            ctx.repaint_after(STREAM_TICK);
        }
        if let Some(deadline) = self.hint_deadline.get() {
            if Instant::now() < deadline {
                ctx.repaint_after(deadline.saturating_duration_since(Instant::now()));
            }
        }
    }

    fn dispatch_event(
        &mut self,
        _event: &TuiEvent,
        _event_ctx: &mut TuiEventContext<'_>,
        _app: &AppContext,
    ) -> bool {
        false
    }
}

/// Static `?` shortcuts overlay, laid out with Warp's two-column helper.
fn help_rows() -> Vec<TxRow> {
    // Width is resolved by the container; the two-column helper needs a
    // concrete width, so help content is fixed single-column rows here and
    // the column layout is demonstrated with pre-split label/detail pairs.
    let pairs = [
        (
            "wheel · pgup/pgdn",
            "scroll transcript (↑/↓ scroll when no history)",
        ),
        ("↑/↓", "prompt history · menu navigate · transcript scroll"),
        ("enter", "submit prompt"),
        ("ctrl-j · shift-enter", "newline in prompt"),
        ("←/→ · home/end", "move cursor (across lines)"),
        (
            "alt-b/alt-f · ctrl-u/k/w",
            "word jump · kill to start/end/word",
        ),
        ("/", "slash-command menu"),
        ("tab · ↑/↓ · esc", "menu accept · navigate · close"),
        ("1–9", "answer blocking prompt"),
        (
            "ctrl-e",
            "fold/unfold long shell, diff, and thinking output",
        ),
        ("ctrl-p · ctrl-n", "previous / next session"),
        ("ctrl-o", "new session"),
        ("ctrl-t", "simulate agent activity"),
        ("ctrl-c", "clear input · cancel stream · exit (×2)"),
    ];
    let constraints = TuiTwoColumnConstraints {
        preferred_first_columns: 26,
        minimum_first_columns: 12,
        minimum_second_columns: 10,
        preferred_maximum_second_columns: 40,
        gap_columns: 2,
    };
    let layout = tui_two_column_layout(70, pairs.iter().map(|(a, b)| (*a, *b)), constraints);
    let mut rows = vec![TxRow::Text(vec![(
        format_tui_first_column("Keyboard shortcuts", layout.with_second_visible(false)),
        crate::theme::Theme.brand_primary_style(),
    )])];
    for (first, second) in &pairs {
        rows.push(TxRow::Text(vec![(
            format!("{}{second}", format_tui_first_column(first, layout)),
            crate::theme::Theme.muted_text_style(),
        )]));
    }
    rows.push(TxRow::Text(vec![(
        format_tui_first_column(
            "Full Warp-style menus and mouse selection arrive in later phases.",
            layout.with_second_visible(false),
        ),
        crate::theme::Theme.dim_text_style(),
    )]));
    rows
}

/// The session surface view.
pub struct SessionView {
    backend: Rc<RefCell<dyn Backend>>,
    theme: Theme,
    link: TuiLink,
    /// Real Warp tab strip hosted as an App child view.
    tab_bar: ViewHandle<TuiTabBarView>,
    prompt: Rc<RefCell<PromptState>>,
    /// Submitted-prompt history ring (up/down recall on single-line input).
    history: PromptHistory,
    /// Collapse-all toggle for long shell/diff/thinking output.
    collapse_all: Cell<bool>,
    menu_open: Rc<Cell<bool>>,
    menu_selected: Cell<usize>,
    menu_query: RefCell<String>,
    menu_dismissed_for: RefCell<Option<String>>,
    scroll_rows: usize,
    /// Pinned to the bottom (follows new output). Any upward scroll unpins;
    /// scrolling back to the bottom re-pins.
    scroll_pinned: bool,
    viewport: Rc<Cell<usize>>,
    total_rows: Rc<Cell<usize>>,
    show_help: Cell<bool>,
    exit: Rc<RefCell<ExitConfirmation>>,
    hint: TransientHint,
    /// Display mirror shared with the live statusline element (see its docs
    /// for why the Warp struct itself cannot be shared).
    hint_mirror: Rc<RefCell<Option<(String, TransientHintTone)>>>,
    /// Wall-clock expiry for the transient hint. `TransientHint`'s own timer
    /// future never runs under `TuiRuntime` (no executor driver), so the pump
    /// element enforces the deadline instead — see `TranscriptBody`.
    hint_deadline: Rc<Cell<Option<Instant>>>,
    quit: Rc<Cell<bool>>,
}

/// Interval between stream-pump repaints while a stream is live: fast enough
/// to feel live, slow enough to read word-by-word.
pub(crate) const STREAM_TICK: std::time::Duration = std::time::Duration::from_millis(90);

impl SessionView {
    pub fn new(
        backend: Rc<RefCell<dyn Backend>>,
        tab_bar: ViewHandle<TuiTabBarView>,
        quit: Rc<Cell<bool>>,
    ) -> Self {
        Self {
            backend,
            theme: Theme,
            link: TuiLink::default(),
            tab_bar,
            prompt: Rc::new(RefCell::new(PromptState::new())),
            history: PromptHistory::default(),
            collapse_all: Cell::new(false),
            menu_open: Rc::new(Cell::new(false)),
            menu_selected: Cell::new(0),
            menu_query: RefCell::new(String::new()),
            menu_dismissed_for: RefCell::new(None),
            scroll_rows: 0,
            scroll_pinned: true,
            viewport: Rc::new(Cell::new(24)),
            total_rows: Rc::new(Cell::new(0)),
            show_help: Cell::new(false),
            exit: Rc::new(RefCell::new(ExitConfirmation::default())),
            hint: TransientHint::default(),
            hint_mirror: Rc::new(RefCell::new(None)),
            hint_deadline: Rc::new(Cell::new(None)),
            quit,
        }
    }

    /// Tab-strip configuration derived from backend sessions. Keys are stable
    /// session ids, so the strip survives renames and insertions. Labels are
    /// capped so one long title cannot crowd out its neighbors. The active
    /// tab carries a working dot while the agent runs (per-session busy
    /// isn't exposed by the trait — only the active session's state).
    pub fn tab_config(backend: &dyn Backend) -> TuiTabBarConfig {
        use crate::backend::AgentStatus;
        let summaries = backend.session_summaries();
        let active = backend.active();
        let working = backend.agent_status() == AgentStatus::Working;
        let mut config = TuiTabBarConfig::new(
            summaries
                .iter()
                .enumerate()
                .map(|(index, session)| {
                    let tab = TuiTab::new(session.id.clone(), session.title.clone())
                        .with_max_label_columns(24);
                    if index == active && working {
                        tab.with_trailing_text(" ●", Theme.attention_glyph_style())
                    } else {
                        tab
                    }
                })
                .collect(),
        );
        config.selected_key = summaries
            .get(backend.active())
            .map(|session| session.id.clone());
        config.focused = true;
        config
    }

    /// Push the current backend sessions into the hosted tab strip.
    /// `set_config` itself skips identical configs without notifying.
    fn sync_tabs(&self, ctx: &mut ViewContext<Self>) {
        let config = Self::tab_config(&*self.backend.borrow());
        let tab_bar = self.tab_bar.clone();
        ctx.update_view(&tab_bar, move |tab, tab_ctx| {
            if let Err(error) = tab.set_config(config, tab_ctx) {
                // Config is built from validated backend state (unique id
                // keys, non-empty labels), so failure is a programming error.
                log::warn!("tab config rejected: {error:?}");
            }
        });
    }

    /// Record a transient-hint deadline enforced by the transcript pump
    /// (`TransientHint`'s own timer future never runs under `TuiRuntime`).
    fn arm_hint_deadline(&self) {
        self.hint_deadline
            .set(Some(Instant::now() + TRANSIENT_HINT_DURATION));
    }

    /// Show a transient footer notice and mirror it for the live statusline.
    fn post_hint(&mut self, text: String, tone: TransientHintTone, ctx: &mut ViewContext<Self>) {
        fn project(view: &mut SessionView) -> &mut TransientHint {
            &mut view.hint
        }
        match tone {
            TransientHintTone::Muted => self.hint.show(text, ctx, project),
            TransientHintTone::Success => self.hint.show_success(text, ctx, project),
            TransientHintTone::Error => self.hint.show_error(text, ctx, project),
        }
        self.hint_mirror.replace(
            self.hint
                .current()
                .map(|(text, tone)| (text.to_owned(), tone)),
        );
        self.arm_hint_deadline();
    }

    /// Clear any transient notice (content and mirror).
    fn clear_hint(&mut self) {
        self.hint.clear();
        self.hint_mirror.take();
    }

    fn current_menu(&self) -> Option<SlashMenu> {
        let text = self.prompt.borrow().text();
        let backend = self.backend.borrow();
        let mut query = self.menu_query.borrow_mut();
        // Track the query so selection resets when it changes.
        let next_query = text.strip_prefix('/').unwrap_or("").to_owned();
        if *query != next_query {
            *query = next_query.clone();
            self.menu_selected.set(0);
        }
        let dismissed = self.menu_dismissed_for.borrow();
        SlashMenu::for_buffer(
            &backend.commands(),
            &text,
            dismissed.as_deref(),
            self.menu_selected.get(),
        )
    }

    /// Select the tab whose key matches a session id (tab-strip events).
    pub fn select_tab_by_key(
        &mut self,
        key: &str,
        backend: &Rc<RefCell<dyn Backend>>,
        ctx: &mut ViewContext<Self>,
    ) {
        let index = backend
            .borrow()
            .session_summaries()
            .iter()
            .position(|session| session.id == key);
        if let Some(index) = index {
            backend.borrow_mut().set_active(index);
            self.sync_tabs(ctx);
            self.pin_to_bottom();
            ctx.notify();
        }
    }

    fn max_skip(&self) -> usize {
        self.total_rows
            .get()
            .saturating_sub(self.viewport.get().max(1))
    }

    fn scroll_by(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        let max = self.max_skip();
        let base = if self.scroll_pinned {
            max
        } else {
            self.scroll_rows.min(max)
        };
        let next = (base as i64 + delta as i64).clamp(0, max as i64) as usize;
        self.scroll_rows = next;
        // Scrolling up unpins; reaching the bottom re-pins.
        self.scroll_pinned = next >= max;
    }

    fn pin_to_bottom(&mut self) {
        self.scroll_pinned = true;
    }
}

impl Entity for SessionView {
    type Event = ();
}

impl TuiView for SessionView {
    fn ui_name() -> &'static str {
        "SessionView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn TuiElement> {
        let menu = self.current_menu();
        self.menu_open.set(menu.is_some());
        let shared = ViewShared {
            backend: self.backend.clone(),
            theme: self.theme,
            link: self.link.clone(),
            viewport: self.viewport.clone(),
            total_rows: self.total_rows.clone(),
            hint_deadline: self.hint_deadline.clone(),
        };
        let mut column = TuiFlex::column().child(TuiChildView::new(&self.tab_bar).finish());
        column = column.flex_child(
            TranscriptBody::new(
                &shared,
                self.show_help.get(),
                self.scroll_rows,
                self.scroll_pinned,
                self.collapse_all.get(),
            )
            .finish(),
        );
        if let Some(menu) = &menu {
            column = column.child(render_menu(self.theme, menu));
        }
        column = column.child(
            PromptElement::new(
                self.prompt.clone(),
                self.backend.clone(),
                self.menu_open.clone(),
                self.theme,
            )
            .finish(),
        );
        column = column.child(
            StatuslineElement::new(
                self.backend.clone(),
                self.theme,
                self.hint_mirror.clone(),
                self.hint_deadline.clone(),
                self.exit.clone(),
            )
            .finish(),
        );

        let tree = column.finish();
        let menu_up = self.menu_open.clone();
        let menu_down = self.menu_open.clone();
        TuiEventHandler::new(tree)
            .on_key("c", |event, ctx, _| {
                if is_ctrl(event) {
                    ctx.dispatch_typed_action(SessionAction::QuitOrCancel);
                }
            })
            .on_key("ctrl-c", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::QuitOrCancel);
            })
            .on_key("t", |event, ctx, _| {
                if is_ctrl(event) {
                    ctx.dispatch_typed_action(SessionAction::SimulateActivity);
                }
            })
            .on_key("p", |event, ctx, _| {
                if is_ctrl(event) {
                    ctx.dispatch_typed_action(SessionAction::CycleTab(-1));
                }
            })
            .on_key("n", |event, ctx, _| {
                if is_ctrl(event) {
                    ctx.dispatch_typed_action(SessionAction::CycleTab(1));
                }
            })
            .on_key("o", |event, ctx, _| {
                if is_ctrl(event) {
                    ctx.dispatch_typed_action(SessionAction::NewSession);
                }
            })
            .on_key("pageup", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::Scroll(ScrollDelta::Page(-1)));
            })
            .on_key("pagedown", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::Scroll(ScrollDelta::Page(1)));
            })
            .on_key("up", move |_, ctx, _| {
                if menu_up.get() {
                    ctx.dispatch_typed_action(SessionAction::Menu(MenuNav::Prev));
                } else {
                    ctx.dispatch_typed_action(SessionAction::History(-1));
                }
            })
            .on_key("down", move |_, ctx, _| {
                if menu_down.get() {
                    ctx.dispatch_typed_action(SessionAction::Menu(MenuNav::Next));
                } else {
                    ctx.dispatch_typed_action(SessionAction::History(1));
                }
            })
            .on_key("?", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::ToggleHelp);
            })
            .on_key("enter", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::Menu(MenuNav::Accept));
            })
            .on_key("escape", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::DismissTop);
            })
            .on_key("tab", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::Menu(MenuNav::Accept));
            })
            .on_key("e", |event, ctx, _| {
                if is_ctrl(event) {
                    ctx.dispatch_typed_action(SessionAction::ToggleCollapse);
                }
            })
            .on_key("1", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::AnswerQuestion(0));
            })
            .on_key("2", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::AnswerQuestion(1));
            })
            .on_key("3", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::AnswerQuestion(2));
            })
            .on_key("4", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::AnswerQuestion(3));
            })
            .on_key("5", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::AnswerQuestion(4));
            })
            .on_key("6", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::AnswerQuestion(5));
            })
            .on_key("7", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::AnswerQuestion(6));
            })
            .on_key("8", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::AnswerQuestion(7));
            })
            .on_key("9", |_, ctx, _| {
                ctx.dispatch_typed_action(SessionAction::AnswerQuestion(8));
            })
            .finish()
    }
}

/// Whether a key event carries the ctrl modifier (root bindings registered by
/// bare key name also observe ctrl-combos, since matching is modifier-blind).
fn is_ctrl(event: &TuiEvent) -> bool {
    matches!(
        event,
        TuiEvent::KeyDown { keystroke, .. } if keystroke.ctrl
    )
}

impl TypedActionView for SessionView {
    type Action = SessionAction;

    fn handle_action(&mut self, action: &SessionAction, ctx: &mut ViewContext<Self>) {
        // Lazily retire a lapsed exit-confirmation window (no timer driver).
        if self.exit.borrow().is_armed() && !self.exit.borrow().should_exit(Instant::now()) {
            self.exit.borrow_mut().disarm();
        }
        match action {
            SessionAction::Prompt(edit) => {
                // Any buffer edit except vertical moves leaves history
                // navigation (a diverged buffer is fresh input).
                match edit {
                    PromptEdit::Up | PromptEdit::Down => {}
                    _ => self.history.reset(),
                }
                let mut prompt = self.prompt.borrow_mut();
                match edit {
                    PromptEdit::Insert(text) => prompt.insert(text),
                    PromptEdit::Newline => prompt.insert_newline(),
                    PromptEdit::Backspace => prompt.backspace(),
                    PromptEdit::Delete => prompt.delete(),
                    PromptEdit::Left => prompt.move_left(),
                    PromptEdit::Right => prompt.move_right(),
                    PromptEdit::Up => {
                        prompt.move_up();
                    }
                    PromptEdit::Down => {
                        prompt.move_down();
                    }
                    PromptEdit::Home => prompt.move_home(),
                    PromptEdit::End => prompt.move_end(),
                    PromptEdit::WordBack => prompt.move_word_back(),
                    PromptEdit::WordForward => prompt.move_word_forward(),
                    PromptEdit::KillToStart => prompt.kill_to_start(),
                    PromptEdit::KillToEnd => prompt.kill_to_end(),
                    PromptEdit::KillWordBack => prompt.kill_word_back(),
                }
                ctx.notify();
            }
            SessionAction::Submit => {
                self.clear_hint();
                if self.prompt.borrow().text().trim().is_empty() {
                    self.prompt.borrow_mut().clear();
                    self.post_hint(
                        "Nothing to submit — type a message or / command".to_owned(),
                        TransientHintTone::Error,
                        ctx,
                    );
                    return;
                }
                let text = self.prompt.borrow().text();
                self.history.push(&text);
                self.prompt.borrow_mut().clear();
                self.menu_dismissed_for.borrow_mut().take();
                self.backend.borrow_mut().submit(text);
                self.show_help.set(false);
                self.pin_to_bottom();
                ctx.notify();
            }
            SessionAction::Menu(nav) => {
                match nav {
                    MenuNav::Prev => {
                        self.menu_selected
                            .set(self.menu_selected.get().saturating_sub(1));
                    }
                    MenuNav::Next => {
                        self.menu_selected.set(self.menu_selected.get() + 1);
                    }
                    MenuNav::Accept => {
                        if let Some(menu) = self.current_menu() {
                            if let Some(cmd) = menu.selected_command() {
                                let mut prompt = self.prompt.borrow_mut();
                                prompt.clear();
                                prompt.insert(&format!("/{} ", cmd.name));
                                self.menu_dismissed_for.borrow_mut().take();
                            }
                        }
                    }
                }
                ctx.notify();
            }
            SessionAction::Scroll(delta) => {
                match *delta {
                    ScrollDelta::Lines(n) => self.scroll_by(n),
                    ScrollDelta::Page(dir) => {
                        self.scroll_by(dir * self.viewport.get().max(1) as i32)
                    }
                }
                ctx.notify();
            }
            SessionAction::History(delta) => {
                // Warp-style recall: single-line input navigates submitted
                // prompts; every other mode keeps scrolling (live gate,
                // multiline buffer, empty ring all fall through).
                let single_line = self.prompt.borrow().lines.len() <= 1;
                let gate_live = self.backend.borrow().blocker().is_some();
                if single_line && !gate_live {
                    let current = self.prompt.borrow().text();
                    if let Some(recall) = self.history.navigate(&current, *delta) {
                        self.prompt.borrow_mut().set_text(&recall);
                        ctx.notify();
                        return;
                    }
                }
                let lines = if *delta < 0 { -1 } else { 1 };
                self.scroll_by(lines);
                ctx.notify();
            }
            SessionAction::ToggleCollapse => {
                let collapsed = !self.collapse_all.get();
                self.collapse_all.set(collapsed);
                self.post_hint(
                    if collapsed {
                        "Folded long output — ctrl-e to expand".to_owned()
                    } else {
                        "Expanded long output".to_owned()
                    },
                    TransientHintTone::Muted,
                    ctx,
                );
            }
            SessionAction::QuitOrCancel => {
                if !self.prompt.borrow().is_empty() {
                    // First ctrl-c clears the input (Warp's contextual action).
                    self.prompt.borrow_mut().clear();
                    self.exit.borrow_mut().disarm();
                } else if self.backend.borrow().stream_active() {
                    // A running stream is cancelled before exit is considered.
                    self.backend.borrow_mut().cancel();
                    self.exit.borrow_mut().disarm();
                    self.post_hint("Cancelled".to_owned(), TransientHintTone::Muted, ctx);
                    return;
                } else if self.exit.borrow().should_exit(Instant::now()) {
                    self.quit.set(true);
                } else {
                    self.exit.borrow_mut().arm(Instant::now());
                    self.post_hint(
                        "Press ctrl-c again to exit".to_owned(),
                        TransientHintTone::Muted,
                        ctx,
                    );
                    return;
                }
                ctx.notify();
            }
            SessionAction::ToggleHelp => {
                let show = !self.show_help.get();
                self.show_help.set(show);
                if show {
                    self.scroll_rows = 0;
                } else {
                    self.pin_to_bottom();
                }
                ctx.notify();
            }
            SessionAction::DismissTop => {
                if self.show_help.get() {
                    self.show_help.set(false);
                    self.pin_to_bottom();
                } else if let Some(menu) = self.current_menu() {
                    *self.menu_dismissed_for.borrow_mut() = Some(menu.query);
                }
                ctx.notify();
            }
            SessionAction::LinkOpened(url) => {
                self.post_hint(format!("link: {url}"), TransientHintTone::Success, ctx);
                ctx.notify();
            }
            SessionAction::AnswerQuestion(option) => {
                // Digits only resolve a live gate; otherwise they are text
                // (the prompt element consumes them before they reach here).
                let blocker = self.backend.borrow().blocker();
                match blocker {
                    Some(Blocker::Permission) => {
                        self.backend.borrow_mut().resolve_permission(*option == 0);
                        self.pin_to_bottom();
                        ctx.notify();
                    }
                    Some(Blocker::Question) => {
                        self.backend.borrow_mut().answer_question(*option);
                        self.pin_to_bottom();
                        ctx.notify();
                    }
                    None => {}
                }
            }
            SessionAction::SimulateActivity => {
                self.backend.borrow_mut().simulate_activity();
                self.pin_to_bottom();
                ctx.notify();
            }
            SessionAction::CycleTab(delta) => {
                // Resolve the adjacent tab through the strip itself so
                // wraparound and non-selectable tabs behave like Warp.
                let direction = if *delta < 0 {
                    TuiTabBarNavigationDirection::Previous
                } else {
                    TuiTabBarNavigationDirection::Next
                };
                let tab_bar = self.tab_bar.clone();
                let target: Option<String> =
                    ctx.update_view(&tab_bar, move |tab, _| tab.navigation_target(direction));
                if let Some(key) = target {
                    let backend = self.backend.clone();
                    self.select_tab_by_key(&key, &backend, ctx);
                }
                ctx.notify();
            }
            SessionAction::NewSession => {
                let n = self.backend.borrow().session_summaries().len() + 1;
                let id = self
                    .backend
                    .borrow_mut()
                    .new_session(format!("session {n}"));
                let index = self
                    .backend
                    .borrow()
                    .session_summaries()
                    .iter()
                    .position(|session| session.id == id)
                    .unwrap_or(0);
                self.backend.borrow_mut().set_active(index);
                self.sync_tabs(ctx);
                self.pin_to_bottom();
                ctx.notify();
            }
        }
    }
}
