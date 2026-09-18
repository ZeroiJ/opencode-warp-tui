//! Inline slash-command menu (mock data, Warp interaction pattern).
//!
//! Warp shows the menu above the input while the buffer starts with `/`,
//! filters by the typed query, highlights the selection with a solid cyan
//! background, and closes on exact match (`should_close_…_for_exact_match`).

use warpui_core::elements::tui::{TuiContainer, TuiElement, TuiFlex, TuiText};

use crate::backend::SlashCommand;
use crate::theme::Theme;

/// Visible menu state, derived per frame from the prompt buffer.
pub struct SlashMenu {
    pub query: String,
    pub items: Vec<SlashCommand>,
    pub selected: usize,
}

impl SlashMenu {
    /// Build from the backend command list; `None` when the menu is closed
    /// (buffer doesn't start with `/`, dismissed query, or exact match).
    pub fn for_buffer(
        commands: &[SlashCommand],
        buffer: &str,
        dismissed_query: Option<&str>,
        selected: usize,
    ) -> Option<Self> {
        let query = buffer.strip_prefix('/')?.to_owned();
        if query.contains(char::is_whitespace) {
            return None;
        }
        if Some(query.as_str()) == dismissed_query {
            return None;
        }
        let items: Vec<SlashCommand> = commands
            .iter()
            .filter(|cmd| cmd.name.starts_with(query.as_str()))
            .cloned()
            .collect();
        if items.is_empty() {
            return None;
        }
        // Exact match submits directly instead of lingering the menu.
        if items.len() == 1 && items[0].name == query {
            return None;
        }
        let selected = selected.min(items.len().saturating_sub(1));
        Some(Self {
            query,
            items,
            selected,
        })
    }

    pub fn selected_command(&self) -> Option<&SlashCommand> {
        self.items.get(self.selected)
    }
}

/// Render the menu card above the input.
pub fn render_menu(theme: Theme, menu: &SlashMenu) -> Box<dyn TuiElement> {
    let bg = theme.menu_background();
    let mut column = TuiFlex::column();
    for (i, item) in menu.items.iter().enumerate() {
        let row = if i == menu.selected {
            let selected = theme.selection_text_style();
            TuiText::from_spans([
                (format!("  /{} ", item.name), selected),
                (item.description.clone(), selected),
            ])
            .truncate()
            .finish()
        } else {
            TuiText::from_spans([
                (format!("  /{} ", item.name), theme.link_text_style().bg(bg)),
                (item.description.clone(), theme.muted_text_style().bg(bg)),
            ])
            .truncate()
            .finish()
        };
        column = column.child(row);
    }
    TuiContainer::new(column.finish())
        .with_border()
        .with_border_style(theme.accent_border_style())
        .with_background(bg)
        .finish()
}

#[cfg(test)]
mod tests {
    use super::SlashMenu;
    use crate::backend::SlashCommand;

    fn commands() -> Vec<SlashCommand> {
        ["build", "test"]
            .map(|name| SlashCommand {
                name: name.into(),
                description: "d".into(),
            })
            .into_iter()
            .collect()
    }

    #[test]
    fn filters_by_prefix() {
        let menu = SlashMenu::for_buffer(&commands(), "/t", None, 0).unwrap();
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].name, "test");
    }

    #[test]
    fn closed_without_slash_or_on_exact_match() {
        assert!(SlashMenu::for_buffer(&commands(), "hello", None, 0).is_none());
        assert!(SlashMenu::for_buffer(&commands(), "/test", None, 0).is_none());
        assert!(SlashMenu::for_buffer(&commands(), "/zzz", None, 0).is_none());
    }

    #[test]
    fn dismissed_query_stays_closed() {
        assert!(SlashMenu::for_buffer(&commands(), "/t", Some("t"), 0).is_none());
        assert!(SlashMenu::for_buffer(&commands(), "/te", Some("t"), 0).is_some());
    }
}
