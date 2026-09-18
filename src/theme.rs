//! Local theme seam replacing Warp's `TuiUiBuilder`.
//!
//! Warp derives every TUI style from the desktop `WarpTheme` plus a probed
//! terminal background (`crates/warp_tui/src/tui_builder.rs`). That theme
//! engine is Warp-application code and was deliberately not copied. This
//! module re-implements the same *semantic* style vocabulary (primary, muted,
//! accent, success, …) over a hardcoded dark palette so views keep asking for
//! "muted text" instead of raw colors.
//!
//! Palette values are documented approximations of Warp's default dark theme
//! and standard dark ANSI slots — not measured values. Exact theme porting
//! (including light-scheme and terminal-background probing) is Phase-8 work.
//! Two literals are exact: the brand lilac `#D2B5FF` and brand green
//! `#E2FFD4` (LightOnDark `design_palette`, `tui_builder.rs`).

use warpui_core::elements::tui::{Color, Modifier, TuiStyle};

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// Semantic style provider for the TUI. Cheap to copy; construct per render.
#[derive(Clone, Copy, Debug, Default)]
pub struct Theme;

impl Theme {
    /// Primary response/body text (theme foreground, main strength).
    pub fn primary_text_style(self) -> TuiStyle {
        TuiStyle::default().fg(rgb(0xE6, 0xED, 0xF3))
    }

    /// Muted secondary text (thinking bodies, footer metadata).
    pub fn muted_text_style(self) -> TuiStyle {
        TuiStyle::default().fg(rgb(0x8B, 0x94, 0x9E))
    }

    /// De-emphasized rows (stubs, placeholders).
    pub fn dim_text_style(self) -> TuiStyle {
        self.muted_text_style().add_modifier(Modifier::DIM)
    }

    /// Full-strength accent (prompt `>` marker, question glyphs).
    pub fn accent_text_style(self) -> TuiStyle {
        TuiStyle::default().fg(rgb(0x39, 0xC5, 0xCF))
    }

    /// Bold accent prompt marker over the input background.
    pub fn input_prefix_style(self) -> TuiStyle {
        self.accent_text_style()
            .bg(self.input_background())
            .add_modifier(Modifier::BOLD)
    }

    /// Accent-tinted background behind the input row.
    pub fn input_background(self) -> Color {
        rgb(0x17, 0x25, 0x2B)
    }

    /// Bold input text over the input background.
    pub fn input_text_style(self) -> TuiStyle {
        TuiStyle::default()
            .fg(rgb(0xE6, 0xED, 0xF3))
            .bg(self.input_background())
            .add_modifier(Modifier::BOLD)
    }

    /// Error text and failed glyphs (terminal red).
    pub fn error_text_style(self) -> TuiStyle {
        TuiStyle::default().fg(rgb(0xF8, 0x51, 0x49))
    }

    /// Completed-tool `✓` (terminal green).
    pub fn success_glyph_style(self) -> TuiStyle {
        TuiStyle::default().fg(rgb(0x3F, 0xB9, 0x50))
    }

    /// Running/approval-blocked glyphs (terminal yellow).
    pub fn attention_glyph_style(self) -> TuiStyle {
        TuiStyle::default().fg(rgb(0xD2, 0x99, 0x22))
    }

    /// Added diff lines and `+n` counts.
    pub fn diff_added_style(self) -> TuiStyle {
        TuiStyle::default().fg(rgb(0x3F, 0xB9, 0x50))
    }

    /// Removed diff lines and `−n` counts.
    pub fn diff_removed_style(self) -> TuiStyle {
        TuiStyle::default().fg(rgb(0xF8, 0x51, 0x49))
    }

    /// Linked filenames / URLs (terminal blue).
    pub fn link_text_style(self) -> TuiStyle {
        TuiStyle::default().fg(rgb(0x58, 0xA6, 0xFF))
    }

    /// Shell-command `!` markers (bright green).
    pub fn shell_command_accent_style(self) -> TuiStyle {
        TuiStyle::default()
            .fg(rgb(0x7E, 0xE8, 0x83))
            .add_modifier(Modifier::BOLD)
    }

    /// Pale-green tint behind shell rows.
    pub fn shell_command_background(self) -> Color {
        rgb(0x18, 0x28, 0x1E)
    }

    /// Solid cyan menu-selection background.
    pub fn selection_background(self) -> Color {
        rgb(0x39, 0xC5, 0xCF)
    }

    /// Bold dark text over the selection background.
    pub fn selection_text_style(self) -> TuiStyle {
        TuiStyle::default()
            .fg(rgb(0x0B, 0x14, 0x16))
            .bg(self.selection_background())
            .add_modifier(Modifier::BOLD)
    }

    /// Accent border for focused/primary containers (dimmed cyan).
    pub fn accent_border_style(self) -> TuiStyle {
        TuiStyle::default().fg(rgb(0x1F, 0x6F, 0x76))
    }

    /// Lilac brand titles/progress (exact Warp LightOnDark literal).
    pub fn brand_primary_style(self) -> TuiStyle {
        TuiStyle::default().fg(rgb(0xD2, 0xB5, 0xFF))
    }

    /// Green brand prompts/actions (exact Warp LightOnDark literal).
    pub fn brand_accent_style(self) -> TuiStyle {
        TuiStyle::default().fg(rgb(0xE2, 0xFF, 0xD4))
    }

    /// Cyan-tinted card background for read-only menus.
    pub fn menu_background(self) -> Color {
        rgb(0x17, 0x25, 0x2B)
    }

    /// Blue-tinted background for plan bodies.
    pub fn plan_background(self) -> Color {
        rgb(0x1B, 0x26, 0x38)
    }
}
