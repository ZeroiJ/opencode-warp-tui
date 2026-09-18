//! Statusline: context/model on the left, hints on the right.
//!
//! The two ends share one row via flex layout (the filler expands), so the
//! line stays correct at any terminal width without measuring it upfront.
//!
//! The element re-reads backend snapshots in `layout`, like the transcript
//! body: repaints reuse the cached view tree, so anything time-sensitive
//! (stream status, transient hints, exit confirmation) must be live here
//! rather than baked in at view-render time.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use instant::Instant;
use warpui_core::elements::tui::{
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiFlex, TuiLayoutContext,
    TuiPaintContext, TuiPaintSurface, TuiScreenPosition, TuiSize, TuiStyle, TuiText,
};
use warpui_core::AppContext;

use super::widgets::ExitConfirmation;
use super::widgets::{long_running_command_hint, TransientHintTone};
use crate::backend::{AgentStatus, Backend, StatusInfo};
use crate::theme::Theme;

/// Live statusline element over backend + hint state.
pub struct StatuslineElement {
    backend: Rc<RefCell<dyn Backend>>,
    theme: Theme,
    /// Display mirror of the owning view's [`TransientHint`](super::widgets::TransientHint):
    /// the Warp struct cannot be shared by reference (its timer projection
    /// closures need view-owned access), so the view mirrors display content
    /// here on every show/clear.
    hint_mirror: Rc<RefCell<Option<(String, TransientHintTone)>>>,
    hint_deadline: Rc<Cell<Option<Instant>>>,
    exit: Rc<RefCell<ExitConfirmation>>,
    child: Option<Box<dyn TuiElement>>,
}

impl StatuslineElement {
    pub fn new(
        backend: Rc<RefCell<dyn Backend>>,
        theme: Theme,
        hint_mirror: Rc<RefCell<Option<(String, TransientHintTone)>>>,
        hint_deadline: Rc<Cell<Option<Instant>>>,
        exit: Rc<RefCell<ExitConfirmation>>,
    ) -> Self {
        Self {
            backend,
            theme,
            hint_mirror,
            hint_deadline,
            exit,
            child: None,
        }
    }

    fn footer_hint(&self) -> (String, TuiStyle) {
        // The armed window lapses by wall clock; `TuiRuntime` runs no timers,
        // so expiry is evaluated lazily here and on the next action.
        if self.exit.borrow().should_exit(Instant::now()) {
            return (
                "Press ctrl-c again to exit".to_owned(),
                self.theme.attention_glyph_style(),
            );
        }
        if let Some((text, tone)) = self.hint_mirror.borrow().clone() {
            // Display is gated by the pump-enforced deadline: stale content
            // is ignored rather than cleared.
            let live = self
                .hint_deadline
                .get()
                .is_some_and(|deadline| Instant::now() < deadline);
            if live {
                let style = match tone {
                    TransientHintTone::Muted => self.theme.muted_text_style(),
                    TransientHintTone::Success => self.theme.success_glyph_style(),
                    TransientHintTone::Error => self.theme.error_text_style(),
                };
                return (text, style);
            }
        }
        let backend = self.backend.borrow();
        // While the agent works, the footer names the attach key (Warp's
        // long-running-command input-slot hint).
        if backend.agent_status() == AgentStatus::Working {
            if let Some(extra) = long_running_command_hint(Some("ctrl-t")) {
                return (extra, self.theme.dim_text_style());
            }
        }
        let hint = "/ commands · ? shortcuts · ctrl-t activity".to_owned();
        (hint, self.theme.dim_text_style())
    }
}

impl TuiElement for StatuslineElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        // Stream pump lives here (not in the transcript body): flex lays out
        // fixed children before flex children, so polling first guarantees
        // the transcript rows and footer below observe the same post-poll
        // state in one frame — no stale working indicator on drain.
        // Cadence comes from the transcript's repaint requests (~90ms live).
        self.backend.borrow_mut().poll_stream();
        let info: StatusInfo = self.backend.borrow().status();
        let (hint_text, hint_style) = self.footer_hint();
        let left = TuiText::from_spans([
            (format!("{} ", info.model), self.theme.muted_text_style()),
            (
                format!("{} ({}) ", info.cwd, info.branch),
                self.theme.dim_text_style(),
            ),
            (
                format!("ctx {}%", info.context_pct),
                self.theme.muted_text_style(),
            ),
            (
                if info.status == AgentStatus::Working {
                    " ●".to_owned()
                } else {
                    String::new()
                },
                self.theme.attention_glyph_style(),
            ),
        ])
        .truncate()
        .finish();
        let filler = TuiText::new(" ").finish();
        let right = TuiText::new(hint_text)
            .with_style(hint_style)
            .truncate()
            .finish();
        let mut row = TuiFlex::row()
            .child(left)
            .flex_child(filler)
            .child(right)
            .finish();
        let size = row.layout(constraint, ctx, app);
        self.child = Some(row);
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
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        if let Some(child) = &mut self.child {
            return child.dispatch_event(event, event_ctx, app);
        }
        false
    }
}
