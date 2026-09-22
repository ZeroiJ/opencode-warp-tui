//! Prompt state and the prompt element.
//!
//! Mirrors Warp's prompt contract (`crates/warp_tui/src/input/view.rs`):
//! a `>` accent prefix (`!` + shell-green in shell mode, i.e. when the buffer
//! starts with `!`), bold input text over a tinted background, and a hardware
//! terminal cursor. The buffer is a small multiline text model (Phase 3);
//! Warp's full editor (selection, vim, completion) stays out of scope.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use warpui_core::elements::tui::{
    text_width, Modifier, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiFlex,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiScreenPoint, TuiScreenPosition, TuiSize,
    TuiText, TuiZIndex,
};
use warpui_core::AppContext;

use super::session::{PromptEdit, SessionAction};
use crate::theme::Theme;

/// Drop C0 controls (and DEL) from inserted text, keeping newlines and tabs.
/// Terminals deliver pastes as plain text, so escape sequences can only
/// arrive here if a compromised source emits them — never execute them.
fn sanitize_inserted(text: &str) -> String {
    text.chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .collect()
}

/// Map a wheel delta to transcript scroll rows. The runtime reports scroll-up
/// as a positive second component (`event_conversion.rs`: `ScrollUp → (0, 1)`);
/// scrolling up reveals older rows, i.e. *decreases* the top skip — the same
/// sign as the Up-arrow binding (`ScrollDelta::Lines(-1)`).
pub fn wheel_lines(delta: (isize, isize)) -> i32 {
    (-delta.1).clamp(-6, 6) as i32
}

/// Multiline prompt buffer with a (row, col) char cursor.
#[derive(Clone, Debug, Default)]
pub struct PromptState {
    pub lines: Vec<String>,
    pub row: usize,
    pub col: usize,
    /// First visible char of each rendered row (horizontal scrolling).
    pub h_offsets: Vec<usize>,
}

impl PromptState {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            row: 0,
            col: 0,
            h_offsets: vec![0],
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|line| line.is_empty())
    }

    pub fn is_shell_mode(&self) -> bool {
        self.lines.first().is_some_and(|line| line.starts_with('!'))
    }

    fn line_chars(&self, row: usize) -> Vec<char> {
        self.lines
            .get(row)
            .map(|line| line.chars().collect())
            .unwrap_or_default()
    }

    fn byte_of(&self, row: usize, col: usize) -> usize {
        self.line_chars(row)
            .iter()
            .take(col)
            .map(|c| c.len_utf8())
            .sum()
    }

    pub fn insert(&mut self, text: &str) {
        let clean = sanitize_inserted(text);
        let mut parts = clean.split('\n');
        let first = parts.next().unwrap_or("");
        let byte = self.byte_of(self.row, self.col);
        self.lines[self.row].insert_str(byte, first);
        self.col += first.chars().count();
        for part in parts {
            let at = self.byte_of(self.row, self.col);
            let rest = self.lines[self.row].split_off(at);
            let mut new_line = part.to_owned();
            new_line.push_str(&rest);
            self.row += 1;
            self.col = part.chars().count();
            self.lines.insert(self.row, new_line);
            self.h_offsets.insert(self.row, 0);
        }
    }

    pub fn insert_newline(&mut self) {
        self.insert("\n");
    }

    pub fn backspace(&mut self) {
        if self.col > 0 {
            let byte = self.byte_of(self.row, self.col);
            let prev = self.lines[self.row][..byte]
                .chars()
                .next_back()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.lines[self.row].drain(byte - prev..byte);
            self.col -= 1;
        } else if self.row > 0 {
            let rest = self.lines.remove(self.row);
            self.h_offsets.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].chars().count();
            self.lines[self.row].push_str(&rest);
        }
    }

    pub fn delete(&mut self) {
        let byte = self.byte_of(self.row, self.col);
        if byte < self.lines[self.row].len() {
            let next = self.lines[self.row][byte..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.lines[self.row].drain(byte..byte + next);
        } else if self.row + 1 < self.lines.len() {
            let rest = self.lines.remove(self.row + 1);
            self.h_offsets.remove(self.row + 1);
            self.lines[self.row].push_str(&rest);
        }
    }

    pub fn move_left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.lines[self.row].chars().count();
        }
    }

    pub fn move_right(&mut self) {
        if self.col < self.lines[self.row].chars().count() {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    pub fn move_home(&mut self) {
        self.col = 0;
    }

    pub fn move_end(&mut self) {
        self.col = self.lines[self.row].chars().count();
    }

    /// Returns false when already on the first row (caller falls through to
    /// transcript scrolling).
    pub fn move_up(&mut self) -> bool {
        if self.row == 0 {
            return false;
        }
        self.row -= 1;
        self.col = self.col.min(self.lines[self.row].chars().count());
        true
    }

    /// Returns false when already on the last row.
    pub fn move_down(&mut self) -> bool {
        if self.row + 1 >= self.lines.len() {
            return false;
        }
        self.row += 1;
        self.col = self.col.min(self.lines[self.row].chars().count());
        true
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// Replace the whole buffer with one line, cursor at the end (history
    /// recall). Horizontal offsets reset so the line renders from column 0.
    pub fn set_text(&mut self, text: &str) {
        let clean = sanitize_inserted(text);
        let first = clean.split('\n').next().unwrap_or("").to_owned();
        self.lines = vec![first];
        self.row = 0;
        self.col = self.lines[0].chars().count();
        self.h_offsets = vec![0];
    }

    fn word_start_before(&self) -> usize {
        // Whitespace-delimited words: skip blanks left of the cursor, then
        // the word itself. Simple, predictable, terminal-like.
        let chars: Vec<char> = self.line_chars(self.row);
        let mut col = self.col.min(chars.len());
        while col > 0 && chars[col - 1].is_whitespace() {
            col -= 1;
        }
        while col > 0 && !chars[col - 1].is_whitespace() {
            col -= 1;
        }
        col
    }

    fn word_end_after(&self) -> usize {
        let chars: Vec<char> = self.line_chars(self.row);
        let mut col = self.col.min(chars.len());
        while col < chars.len() && !chars[col].is_whitespace() {
            col += 1;
        }
        while col < chars.len() && chars[col].is_whitespace() {
            col += 1;
        }
        col
    }

    pub fn move_word_back(&mut self) {
        self.col = self.word_start_before();
    }

    pub fn move_word_forward(&mut self) {
        self.col = self.word_end_after();
    }

    /// Delete from the cursor to the start of the current line.
    pub fn kill_to_start(&mut self) {
        let byte = self.byte_of(self.row, self.col);
        self.lines[self.row].drain(..byte);
        self.col = 0;
    }

    /// Delete from the cursor to the end of the current line.
    pub fn kill_to_end(&mut self) {
        let byte = self.byte_of(self.row, self.col);
        self.lines[self.row].drain(byte..);
    }

    /// Delete the word before the cursor (whitespace-delimited).
    pub fn kill_word_back(&mut self) {
        let start = self.word_start_before();
        let from = self
            .line_chars(self.row)
            .iter()
            .take(start)
            .map(|c| c.len_utf8())
            .sum();
        let to = self.byte_of(self.row, self.col);
        self.lines[self.row].drain(from..to);
        self.col = start;
    }
}

/// Submitted-prompt history ring (Warp's up-arrow recall, in-memory only;
/// disk persistence stays a Phase 10 concern).
#[derive(Clone, Debug, Default)]
pub struct PromptHistory {
    entries: Vec<String>,
    /// Position while navigating (`None` = editing fresh input).
    cursor: Option<usize>,
    /// Buffer text stashed when navigation starts (restored past newest).
    draft: String,
}

/// Ring capacity: enough for a long session, small enough to stay trivial.
const HISTORY_CAP: usize = 100;

impl PromptHistory {
    /// Record a submission. Empty/whitespace and repeats of the newest
    /// entry are skipped; overflow drops the oldest.
    pub fn push(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.entries.last().is_some_and(|last| last == trimmed) {
            self.reset();
            return;
        }
        self.entries.push(trimmed.to_owned());
        while self.entries.len() > HISTORY_CAP {
            self.entries.remove(0);
        }
        self.reset();
    }

    /// Leave navigation mode (call on any buffer edit or submit).
    pub fn reset(&mut self) {
        self.cursor = None;
        self.draft.clear();
    }

    /// Step older (`delta < 0`) or newer (`delta > 0`). Returns the buffer
    /// text to show, or `None` when there is no history to navigate (the
    /// caller keeps existing behavior, e.g. transcript scroll).
    pub fn navigate(&mut self, current: &str, delta: i32) -> Option<String> {
        if self.entries.is_empty() || delta == 0 {
            return None;
        }
        if self.cursor.is_none() {
            self.draft = current.to_owned();
            self.cursor = if delta < 0 {
                Some(self.entries.len() - 1)
            } else {
                // Down with no navigation active: nothing newer than the
                // draft — stay put so down-arrow keeps scrolling.
                return None;
            };
        } else if delta < 0 {
            let cursor = self.cursor.unwrap_or(0).saturating_sub(1);
            self.cursor = Some(cursor);
        } else {
            let next = self.cursor.unwrap_or(0) + 1;
            if next >= self.entries.len() {
                let draft = std::mem::take(&mut self.draft);
                self.cursor = None;
                return Some(draft);
            }
            self.cursor = Some(next);
        }
        Some(self.entries[self.cursor.unwrap_or(0)].clone())
    }
}

/// Multiline prompt element: `> buffer` with hardware cursor.
///
/// Text keys are consumed here and forwarded as [`SessionAction::Prompt`];
/// anything else (notably ctrl-combos, and menu/blocker routing keys when
/// those modes are active) falls through to the session-level handler.
///
/// The blocking gate is read live from the backend on every key: gates can
/// arrive via the stream pump without any view re-render, so a cached flag
/// would route digits to the gate after it resolved (or swallow text while
/// one is pending).
pub struct PromptElement {
    state: Rc<RefCell<PromptState>>,
    backend: Rc<RefCell<dyn crate::backend::Backend>>,
    /// When true, enter/escape/up/down/tab belong to the open inline menu
    /// (menus only change on keystrokes, so this render-time flag is fresh).
    menu_open: Rc<Cell<bool>>,
    theme: Theme,
    child: Option<Box<dyn TuiElement>>,
    cursor: (i32, i32),
}

impl PromptElement {
    pub fn new(
        state: Rc<RefCell<PromptState>>,
        backend: Rc<RefCell<dyn crate::backend::Backend>>,
        menu_open: Rc<Cell<bool>>,
        theme: Theme,
    ) -> Self {
        Self {
            state,
            backend,
            menu_open,
            theme,
            child: None,
            cursor: (0, 0),
        }
    }
}

impl TuiElement for PromptElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let width = constraint.max.width;
        let theme = self.theme;
        let (prefix, prefix_style) = {
            let state = self.state.borrow();
            if state.is_shell_mode() {
                (
                    "!",
                    theme
                        .shell_command_accent_style()
                        .bg(theme.input_background()),
                )
            } else {
                (">", theme.input_prefix_style())
            }
        };
        // Prefix cell is marker + one trailing space, matching Warp's padded prefix.
        let prefix_width: usize = 2;
        let avail = (width as usize).saturating_sub(prefix_width).max(1);

        let body_style = theme.input_text_style();
        let cursor_style = body_style.add_modifier(Modifier::REVERSED);
        let mut column = TuiFlex::column();
        let (cursor_x, cursor_y) = {
            let mut state = self.state.borrow_mut();
            while state.h_offsets.len() < state.lines.len() {
                state.h_offsets.push(0);
            }
            let mut cursor = (0i32, 0i32);
            // Clone line data first so offset fix-ups below can mutate state.
            let lines: Vec<String> = state.lines.clone();
            for (row, line) in lines.iter().enumerate() {
                let chars: Vec<char> = line.chars().collect();
                let mut offset = state.h_offsets[row].min(chars.len());
                if row == state.row {
                    if state.col < offset {
                        offset = state.col;
                    }
                    while state.col >= offset + avail {
                        offset += 1;
                    }
                    state.h_offsets[row] = offset;
                    let before: String = chars[offset..state.col.min(chars.len())].iter().collect();
                    cursor.0 = prefix_width as i32 + i32::from(text_width(&before));
                    cursor.1 = row as i32;
                }
                let visible: String = chars.iter().skip(offset).take(avail).collect();
                let visible_chars: Vec<char> = visible.chars().collect();
                let cursor_here = row == state.row;
                let cursor_in_window = state.col - offset;
                let (before, cursor_char, after) = if cursor_here {
                    let before: String = visible_chars[..cursor_in_window.min(visible_chars.len())]
                        .iter()
                        .collect();
                    let cursor_char = visible_chars
                        .get(cursor_in_window)
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| " ".to_owned());
                    let after: String = visible_chars.iter().skip(cursor_in_window + 1).collect();
                    (before, cursor_char, after)
                } else {
                    (visible, String::new(), String::new())
                };
                let text_row = TuiFlex::row()
                    .child(
                        TuiText::new(if row == 0 {
                            format!("{prefix} ")
                        } else {
                            "  ".to_owned()
                        })
                        .with_style(prefix_style)
                        .finish(),
                    )
                    .flex_child(
                        TuiText::from_spans([
                            (before, body_style),
                            (cursor_char, cursor_style),
                            (after, body_style),
                        ])
                        .finish(),
                    )
                    .finish();
                column = column.child(text_row);
            }
            cursor
        };
        self.cursor = (cursor_x, cursor_y);

        let mut column = column.finish();
        let height = self.state.borrow().lines.len().max(1) as u16;
        let size = column.layout(TuiConstraint::tight(TuiSize::new(width, height)), ctx, app);
        self.child = Some(column);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        if let Some(child) = &mut self.child {
            child.render(origin, surface, ctx);
        }
        ctx.set_terminal_cursor(TuiScreenPoint::from_position(
            origin.offset(self.cursor.0, self.cursor.1),
            TuiZIndex::Normal(0),
        ));
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        _app: &AppContext,
    ) -> bool {
        // Mouse wheel scrolls the transcript from anywhere (single scroll
        // region in Phase 3). Sign verified live in tmux (Phase 2).
        if let TuiEvent::ScrollWheel { delta, .. } = event {
            let lines = wheel_lines(*delta);
            if lines != 0 {
                event_ctx.dispatch_typed_action(SessionAction::Scroll(
                    super::session::ScrollDelta::Lines(lines),
                ));
                return true;
            }
            return false;
        }
        if let TuiEvent::Paste { text } = event {
            // Multiline pastes land as real newlines (sanitized on insert).
            event_ctx.dispatch_typed_action(SessionAction::prompt_edit(PromptEdit::Insert(
                text.clone(),
            )));
            return true;
        }
        let TuiEvent::KeyDown {
            keystroke, chars, ..
        } = event
        else {
            return false;
        };
        // Ctrl-j inserts a newline (Enter submits); other modifier combos
        // belong to session-level bindings (ctrl-c, ctrl-t, ctrl-p/n/o).
        if keystroke.ctrl && keystroke.key == "j" {
            event_ctx.dispatch_typed_action(SessionAction::prompt_edit(PromptEdit::Newline));
            return true;
        }
        // Word motion and line kills (readline-style, applied once via the
        // session action — unlike arrows, which the element pre-applies).
        if keystroke.alt && !keystroke.ctrl && !keystroke.cmd {
            let edit = match keystroke.key.as_str() {
                "b" => Some(PromptEdit::WordBack),
                "f" => Some(PromptEdit::WordForward),
                _ => None,
            };
            if let Some(edit) = edit {
                event_ctx.dispatch_typed_action(SessionAction::prompt_edit(edit));
                return true;
            }
        }
        if keystroke.ctrl && !keystroke.alt && !keystroke.cmd {
            let edit = match keystroke.key.as_str() {
                "u" => Some(PromptEdit::KillToStart),
                "k" => Some(PromptEdit::KillToEnd),
                "w" => Some(PromptEdit::KillWordBack),
                _ => None,
            };
            if let Some(edit) = edit {
                event_ctx.dispatch_typed_action(SessionAction::prompt_edit(edit));
                return true;
            }
        }
        if keystroke.ctrl || keystroke.alt || keystroke.cmd {
            return false;
        }
        // Menu and blocker modes reroute their keys to the session handler.
        // Digits only reroute for a live gate (slash queries keep digits).
        let blocker_live = self.backend.borrow().blocker().is_some();
        if self.menu_open.get() || blocker_live {
            match keystroke.key.as_str() {
                "enter" | "escape" | "up" | "down" | "tab" => return false,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" if blocker_live => {
                    return false
                }
                _ => {}
            }
        }
        // A lone `?` on an empty buffer opens the shortcuts overlay instead
        // of typing (Warp's `?` menu); anywhere else it is plain text.
        if keystroke.key == "?" && self.state.borrow().text().is_empty() {
            return false;
        }
        // Shift+enter inserts a newline where the terminal reports it;
        // terminals without keyboard enhancement send plain enter (submit).
        if keystroke.key == "enter" && keystroke.shift {
            event_ctx.dispatch_typed_action(SessionAction::prompt_edit(PromptEdit::Newline));
            return true;
        }
        let action = match keystroke.key.as_str() {
            "enter" => SessionAction::Submit,
            "backspace" => SessionAction::prompt_edit(PromptEdit::Backspace),
            "delete" => SessionAction::prompt_edit(PromptEdit::Delete),
            "left" => SessionAction::prompt_edit(PromptEdit::Left),
            "right" => SessionAction::prompt_edit(PromptEdit::Right),
            "home" => SessionAction::prompt_edit(PromptEdit::Home),
            "end" => SessionAction::prompt_edit(PromptEdit::End),
            "up" => {
                if self.state.borrow_mut().move_up() {
                    SessionAction::prompt_edit(PromptEdit::Up)
                } else {
                    return false;
                }
            }
            "down" => {
                if self.state.borrow_mut().move_down() {
                    SessionAction::prompt_edit(PromptEdit::Down)
                } else {
                    return false;
                }
            }
            "escape" => SessionAction::DismissTop,
            _ => {
                let text: String = sanitize_inserted(chars);
                if text.is_empty() {
                    return false;
                }
                SessionAction::prompt_edit(PromptEdit::Insert(text))
            }
        };
        event_ctx.dispatch_typed_action(action);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use warpui_core::elements::tui::{TuiElement, TuiEvent, TuiEventContext, TuiPoint, TuiScene};
    use warpui_core::event::{KeyEventDetails, ModifiersState};
    use warpui_core::keymap::Keystroke;
    use warpui_core::{App, EntityId, EntityIdMap};

    use super::{sanitize_inserted, wheel_lines, PromptElement, PromptState};
    use crate::backend::Backend as _;
    use crate::theme::Theme;

    fn keystroke(key: &str, ctrl: bool, shift: bool) -> Keystroke {
        Keystroke {
            key: key.to_owned(),
            ctrl,
            shift,
            ..Default::default()
        }
    }

    fn key_event(key: &str, chars: &str, ctrl: bool, shift: bool) -> TuiEvent {
        TuiEvent::KeyDown {
            keystroke: keystroke(key, ctrl, shift),
            chars: chars.to_owned(),
            details: KeyEventDetails::default(),
            is_composing: false,
        }
    }

    fn alt_key_event(key: &str) -> TuiEvent {
        TuiEvent::KeyDown {
            keystroke: Keystroke {
                key: key.to_owned(),
                alt: true,
                ..Default::default()
            },
            chars: String::new(),
            details: KeyEventDetails::default(),
            is_composing: false,
        }
    }

    /// Drive one event through a fresh prompt element, returning whether it
    /// was consumed. Asserts routing only; view-side effects are covered by
    /// live runs and `handle_action` behavior.
    fn dispatch(
        event: &TuiEvent,
        menu_open: bool,
        backend: crate::backend::mock::MockBackend,
        app_ctx: &warpui_core::AppContext,
    ) -> bool {
        let state = Rc::new(RefCell::new(PromptState::new()));
        let mut element = PromptElement::new(
            state,
            Rc::new(RefCell::new(backend)),
            Rc::new(Cell::new(menu_open)),
            Theme,
        );
        let scene = Rc::new(TuiScene::default());
        let mut views = EntityIdMap::default();
        let mut event_ctx = TuiEventContext::new(scene, &mut views);
        event_ctx.set_origin_view(Some(EntityId::new()));
        element.dispatch_event(event, &mut event_ctx, app_ctx)
    }

    fn fresh_backend() -> crate::backend::mock::MockBackend {
        crate::backend::mock::MockBackend::demo()
    }

    fn with_app(check: fn(&warpui_core::AppContext)) {
        App::test((), |app| async move {
            app.read(check);
        });
    }

    #[test]
    fn printable_chars_are_consumed() {
        with_app(|app_ctx| {
            assert!(dispatch(
                &key_event("a", "a", false, false),
                false,
                fresh_backend(),
                app_ctx
            ));
        });
    }

    #[test]
    fn ctrl_combos_fall_through_to_session() {
        with_app(|app_ctx| {
            assert!(!dispatch(
                &key_event("c", "", true, false),
                false,
                fresh_backend(),
                app_ctx
            ));
            assert!(!dispatch(
                &key_event("t", "", true, false),
                false,
                fresh_backend(),
                app_ctx
            ));
        });
    }

    #[test]
    fn menu_mode_reroutes_menu_keys() {
        with_app(|app_ctx| {
            for key in ["enter", "escape", "up", "down", "tab"] {
                assert!(
                    !dispatch(
                        &key_event(key, "", false, false),
                        true,
                        fresh_backend(),
                        app_ctx
                    ),
                    "{key} should reach the session menu handler"
                );
            }
            // Ordinary text still edits the query.
            assert!(dispatch(
                &key_event("x", "x", false, false),
                true,
                fresh_backend(),
                app_ctx
            ));
        });
    }

    #[test]
    fn blocker_mode_reroutes_digits_and_enter() {
        with_app(|app_ctx| {
            // A gate pushed through the stream (no view render involved)
            // must still reroute digits: the element reads it live.
            let mut backend = fresh_backend();
            backend.run_scenario(crate::backend::mock::Scenario::Permission);
            while backend.poll_stream() {}
            for key in ["1", "enter", "escape"] {
                assert!(
                    !dispatch(&key_event(key, "", false, false), false, backend, app_ctx),
                    "{key} should reach the blocking gate"
                );
                // Rebuild: dispatch consumes the backend by value.
                backend = fresh_backend();
                backend.run_scenario(crate::backend::mock::Scenario::Permission);
                while backend.poll_stream() {}
            }
        });
    }

    #[test]
    fn shift_enter_and_ctrl_j_are_consumed_as_newline() {
        with_app(|app_ctx| {
            assert!(dispatch(
                &key_event("enter", "", false, true),
                false,
                fresh_backend(),
                app_ctx
            ));
            assert!(dispatch(
                &key_event("j", "", true, false),
                false,
                fresh_backend(),
                app_ctx
            ));
        });
    }

    #[test]
    fn wheel_and_paste_are_consumed() {
        with_app(|app_ctx| {
            let wheel = TuiEvent::ScrollWheel {
                position: TuiPoint::new(50, 15),
                delta: (0, 1),
                precise: false,
                modifiers: ModifiersState::default(),
            };
            assert!(dispatch(&wheel, false, fresh_backend(), app_ctx));
            let paste = TuiEvent::Paste {
                text: "a\nb".to_owned(),
            };
            assert!(dispatch(&paste, false, fresh_backend(), app_ctx));
        });
    }

    #[test]
    fn word_and_kill_keys_are_consumed() {
        with_app(|app_ctx| {
            // alt-b / alt-f word motion.
            assert!(dispatch(
                &alt_key_event("b"),
                false,
                fresh_backend(),
                app_ctx
            ));
            assert!(dispatch(
                &alt_key_event("f"),
                false,
                fresh_backend(),
                app_ctx
            ));
            // ctrl-u / ctrl-k / ctrl-w line kills.
            for key in ["u", "k", "w"] {
                assert!(
                    dispatch(
                        &key_event(key, "", true, false),
                        false,
                        fresh_backend(),
                        app_ctx
                    ),
                    "{key} should be consumed as a line kill"
                );
            }
            // Other ctrl combos still fall through to the session handler.
            assert!(!dispatch(
                &key_event("x", "", true, false),
                false,
                fresh_backend(),
                app_ctx
            ));
            // Other alt combos fall through untouched.
            assert!(!dispatch(
                &alt_key_event("x"),
                false,
                fresh_backend(),
                app_ctx
            ));
        });
    }

    #[test]
    fn insert_and_move() {
        let mut prompt = PromptState::new();
        prompt.insert("hi");
        prompt.insert("!");
        assert_eq!(prompt.text(), "hi!");
        assert_eq!(prompt.col, 3);
        prompt.move_left();
        prompt.move_left();
        prompt.insert("X");
        assert_eq!(prompt.text(), "hXi!");
        assert_eq!(prompt.col, 2);
    }

    #[test]
    fn backspace_and_delete() {
        let mut prompt = PromptState::new();
        prompt.insert("abc");
        prompt.move_left();
        prompt.backspace();
        assert_eq!(prompt.text(), "ac");
        prompt.move_home();
        prompt.delete();
        assert_eq!(prompt.text(), "c");
        prompt.backspace();
        assert_eq!(prompt.text(), "c");
    }

    #[test]
    fn shell_mode_prefix() {
        let mut prompt = PromptState::new();
        prompt.insert("!ls");
        assert!(prompt.is_shell_mode());
        prompt.clear();
        assert!(!prompt.is_shell_mode());
        assert_eq!(prompt.col, 0);
    }

    #[test]
    fn multiline_split_and_join() {
        let mut prompt = PromptState::new();
        prompt.insert("ab\ncd");
        assert_eq!(prompt.lines, vec!["ab".to_owned(), "cd".to_owned()]);
        assert_eq!((prompt.row, prompt.col), (1, 2));
        prompt.move_home();
        prompt.backspace();
        assert_eq!(prompt.lines, vec!["abcd".to_owned()]);
        assert_eq!((prompt.row, prompt.col), (0, 2));
    }

    #[test]
    fn vertical_moves_fall_through_at_edges() {
        let mut prompt = PromptState::new();
        prompt.insert("a\nb");
        assert!(!prompt.move_down());
        prompt.move_up();
        assert!(!prompt.move_up());
        assert!(prompt.move_down());
    }

    #[test]
    fn sanitize_drops_controls_keeps_newlines() {
        assert_eq!(sanitize_inserted("a\x1bb\nc\x7fd"), "ab\ncd");
        assert_eq!(sanitize_inserted("a\tb"), "a\tb");
    }

    #[test]
    fn wheel_up_decreases_top_skip() {
        // Runtime convention: ScrollUp arrives as (0, +1) and must match the
        // Up-arrow binding (Lines(-1), toward older rows).
        assert!(wheel_lines((0, 1)) < 0);
        assert!(wheel_lines((0, -1)) > 0);
    }

    #[test]
    fn history_ring_push_dedups_and_caps() {
        use super::PromptHistory;
        let mut history = PromptHistory::default();
        assert_eq!(history.navigate("", -1), None);
        history.push("  ");
        history.push("one");
        history.push("one");
        history.push("two");
        assert_eq!(history.navigate("", -1), Some("two".to_owned()));
        assert_eq!(history.navigate("", -1), Some("one".to_owned()));
        // Past the oldest: stays on the oldest entry.
        assert_eq!(history.navigate("", -1), Some("one".to_owned()));
        // Past the newest: restores the stashed draft ("" here, stashed
        // when navigation started).
        assert_eq!(history.navigate("", 1), Some("two".to_owned()));
        assert_eq!(history.navigate("ignored", 1), Some(String::new()));
        // Draft restore exits navigation; older starts from the top again.
        assert_eq!(history.navigate("", -1), Some("two".to_owned()));
        history.reset();
        assert_eq!(history.navigate("", 1), None);
    }

    #[test]
    fn history_ring_bounded() {
        use super::{PromptHistory, HISTORY_CAP};
        let mut history = PromptHistory::default();
        for i in 0..(HISTORY_CAP + 10) {
            history.push(&format!("entry {i}"));
        }
        assert_eq!(history.navigate("", -1).as_deref(), Some("entry 109"));
    }

    #[test]
    fn word_motion_and_kills() {
        let mut prompt = PromptState::new();
        prompt.insert("one two three");
        prompt.move_word_back();
        assert_eq!(prompt.col, 8);
        prompt.move_word_back();
        assert_eq!(prompt.col, 4);
        prompt.move_word_forward();
        assert_eq!(prompt.col, 8);
        prompt.kill_word_back();
        assert_eq!(prompt.text(), "one three");
        prompt.move_end();
        prompt.kill_to_start();
        assert_eq!(prompt.text(), "");
        prompt.insert("abc def");
        prompt.move_home();
        prompt.kill_to_end();
        assert_eq!(prompt.text(), "");
        prompt.set_text("hello world");
        assert_eq!(prompt.text(), "hello world");
        assert_eq!(prompt.col, 11);
    }
}
