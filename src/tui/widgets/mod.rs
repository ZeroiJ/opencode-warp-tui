//! Promoted Warp presentation snapshots.
//!
//! Byte-identical to Warp `crates/warp_tui/src/*` at the pinned revision
//! except for the stripped `#[cfg(test)] mod tests;` declarations (the Warp
//! test-harness files were not extracted) and a provenance header line — see
//! `research/phase2.md`. Pristine copies live under `warp-tui/`.
//!
//! `tab_bar` hosts the real Warp tab strip as an App child view (see
//! `SessionView::new`); everything here is used by the session view.

pub(crate) mod exit_confirmation;
pub(crate) mod input_hints;
pub(crate) mod link;
pub(crate) mod osc_notifications;
pub(crate) mod tab_bar;
pub(crate) mod transient_hint;
pub(crate) mod tui_column_layout;

pub(crate) use exit_confirmation::ExitConfirmation;
pub(crate) use input_hints::long_running_command_hint;
pub(crate) use link::TuiLink;
pub(crate) use tab_bar::{
    TuiTab, TuiTabBarConfig, TuiTabBarEvent, TuiTabBarNavigationDirection, TuiTabBarView,
};
pub(crate) use transient_hint::{TransientHint, TransientHintTone, TRANSIENT_HINT_DURATION};
pub(crate) use tui_column_layout::{
    format_tui_first_column, tui_two_column_layout, TuiTwoColumnConstraints,
};

#[cfg(test)]
mod tests {
    use super::tab_bar::{TuiTab, TuiTabBarConfig, TuiTabBarView};

    #[test]
    fn empty_bar_reports_no_tabs() {
        let bar = TuiTabBarView::empty();
        assert!(!bar.has_tabs());
        assert_eq!(bar.selected_key(), None);
    }

    #[test]
    fn configured_bar_reports_selection() {
        let mut config =
            TuiTabBarConfig::new(vec![TuiTab::new("a", "alpha"), TuiTab::new("b", "beta")]);
        config.selected_key = Some("b".into());
        let bar = TuiTabBarView::new(config).expect("unique keys");
        assert!(bar.has_tabs());
        assert_eq!(bar.selected_key(), Some("b"));
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        let config =
            TuiTabBarConfig::new(vec![TuiTab::new("a", "alpha"), TuiTab::new("a", "again")]);
        assert!(TuiTabBarView::new(config).is_err());
    }
}
