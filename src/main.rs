//! `owt` — standalone Warp-style agent TUI.
//!
//! Phase 4: `--backend mock` (default, scripted data) or `--backend opencode`
//! (live OpenCode server via `OpenCodeBackend`). Both implement the same
//! generic [`Backend`](backend::Backend); the TUI cannot tell them apart.

mod backend;
mod theme;
mod tui;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use warpui_core::platform::WindowStyle;
use warpui_core::runtime::TuiRuntime;
use warpui_core::{AddWindowOptions, App};

use backend::mock::MockBackend;
use backend::opencode::{OpenCodeBackend, OpenCodeConfig};
use tui::widgets::TuiTabBarView;
use tui::SessionView;

fn backend_kind() -> String {
    let mut args = std::env::args().skip(1);
    let mut kind = "mock".to_owned();
    while let Some(arg) = args.next() {
        if arg == "--backend" {
            if let Some(value) = args.next() {
                kind = value;
            }
        } else if let Some(value) = arg.strip_prefix("--backend=") {
            kind = value.to_owned();
        }
    }
    kind
}

fn main() {
    let kind = backend_kind();
    let backend: Rc<RefCell<dyn backend::Backend>> = match kind.as_str() {
        "opencode" => match OpenCodeBackend::connect(&OpenCodeConfig::from_env()) {
            Ok(backend) => {
                eprintln!("owt: connected to OpenCode");
                Rc::new(RefCell::new(backend))
            }
            Err(error) => {
                eprintln!("owt: OpenCode connect failed: {error}");
                std::process::exit(2);
            }
        },
        "mock" => Rc::new(RefCell::new(MockBackend::demo())),
        other => {
            eprintln!("owt: unknown --backend {other:?} (want mock|opencode)");
            std::process::exit(2);
        }
    };
    App::test((), |mut app| async move {
        let quit = Rc::new(Cell::new(false));
        let (window_id, root) = app.update(|ctx| {
            let backend = backend.clone();
            let quit = quit.clone();
            ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                move |ctx| {
                    // Host Warp's real tab strip as a child view, seeded from
                    // backend sessions; selection events flow back via the
                    // subscription below (Warp's own parent/child pattern).
                    // An invalid seed (e.g. an empty title) falls back to an
                    // empty strip instead of crashing; the next sync repairs it.
                    let tab_bar = ctx.add_tui_view(|_| {
                        TuiTabBarView::new(SessionView::tab_config(&*backend.borrow()))
                            .unwrap_or_else(|error| {
                                log::warn!("tab seed rejected ({error:?}); starting empty");
                                TuiTabBarView::empty()
                            })
                    });
                    let session = SessionView::new(backend.clone(), tab_bar.clone(), quit);
                    let subscriber_backend = backend.clone();
                    ctx.subscribe_to_view(
                        &tab_bar,
                        move |session: &mut SessionView,
                              _,
                              event: &tui::widgets::TuiTabBarEvent,
                              ctx| {
                            if let tui::widgets::TuiTabBarEvent::SelectTab(key) = event {
                                session.select_tab_by_key(key, &subscriber_backend, ctx);
                            }
                        },
                    );
                    session
                },
            )
        });

        let mut runtime = TuiRuntime::enter(&app, window_id, root).expect("enter alternate screen");
        let quit_for_loop = quit.clone();
        runtime
            .run_until(&mut app, move |_| quit_for_loop.get())
            .expect("run TUI event loop");
    });
}
