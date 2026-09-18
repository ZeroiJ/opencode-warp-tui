//! Transcript rendering: backend blocks → Warp-styled single-row elements.
//!
//! Every block is separated by one blank row (`BLOCK_TOP_PADDING_ROWS`, as in
//! Warp's `transcript_view.rs`). Rows are pre-wrapped to whole lines so the
//! scroll container can skip exact row counts without partial clipping.

use std::time::{Duration, Instant};

use warpui_core::elements::tui::{
    text_width, Modifier, TuiAnimated, TuiElement, TuiStyle, TuiText,
};

use super::session::SessionAction;
use super::widgets::TuiLink;
use crate::backend::{Block, ToolState};
use crate::theme::Theme;

/// Braille spinner frames for running activity (Warp's warping indicator is
/// clock-driven; this is the Phase-2 equivalent over `TuiAnimated`).
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// One transcript row: plain styled text, an interactive link, or an
/// animated activity row.
#[derive(Debug)]
pub enum TxRow {
    Text(Vec<(String, TuiStyle)>),
    Link { label: String, url: String },
    Spinner { label: String },
}

fn text_row(text: impl Into<String>, style: TuiStyle) -> TxRow {
    TxRow::Text(vec![(text.into(), style)])
}

/// Greedy char-level wrap of styled spans into `width` columns.
pub fn wrap_spans(spans: &[(String, TuiStyle)], width: usize) -> Vec<Vec<(String, TuiStyle)>> {
    let width = width.max(1);
    let mut lines: Vec<Vec<(String, TuiStyle)>> = vec![Vec::new()];
    let mut col: usize = 0;
    for (text, style) in spans {
        let mut chunk = String::new();
        let mut chunk_w = 0usize;
        for c in text.chars() {
            if c == '\n' {
                if !chunk.is_empty() {
                    push_chunk(&mut lines, &mut col, std::mem::take(&mut chunk), *style);
                    chunk_w = 0;
                }
                lines.push(Vec::new());
                col = 0;
                continue;
            }
            let w = usize::from(text_width(&c.to_string()));
            // Overflow commits the pending chunk and starts a new visual
            // line (an overlong word still overflows; the row truncates it
            // at paint time).
            if col + chunk_w + w > width && col + chunk_w > 0 {
                push_chunk(&mut lines, &mut col, std::mem::take(&mut chunk), *style);
                chunk_w = 0;
                lines.push(Vec::new());
                col = 0;
            }
            chunk.push(c);
            chunk_w += w;
        }
        if !chunk.is_empty() {
            push_chunk(&mut lines, &mut col, chunk, *style);
        }
    }
    lines
}

fn push_chunk(
    lines: &mut [Vec<(String, TuiStyle)>],
    col: &mut usize,
    chunk: String,
    style: TuiStyle,
) {
    *col += usize::from(text_width(&chunk));
    if let Some(line) = lines.last_mut() {
        line.push((chunk, style));
    }
}

/// Minimal inline markdown: `**bold**`, `` `code` ``, `[label](url)`.
fn inline_spans(theme: Theme, text: &str, base: TuiStyle) -> Vec<(String, TuiStyle)> {
    let bold = base.add_modifier(Modifier::BOLD);
    let code = theme.dim_text_style();
    let link = theme.link_text_style().add_modifier(Modifier::UNDERLINED);
    let mut out = Vec::new();
    let mut rest = text;
    let mut plain = String::new();
    let flush = |plain: &mut String, out: &mut Vec<(String, TuiStyle)>| {
        if !plain.is_empty() {
            out.push((std::mem::take(plain), base));
        }
    };
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("**") {
            if let Some(end) = tail.find("**") {
                flush(&mut plain, &mut out);
                out.push((tail[..end].to_owned(), bold));
                rest = &tail[end + 2..];
                continue;
            }
        }
        if let Some(tail) = rest.strip_prefix('`') {
            if let Some(end) = tail.find('`') {
                flush(&mut plain, &mut out);
                out.push((tail[..end].to_owned(), code));
                rest = &tail[end + 1..];
                continue;
            }
        }
        if rest.starts_with('[') {
            if let Some(mid) = rest.find("](") {
                if let Some(end) = rest[mid..].find(')') {
                    flush(&mut plain, &mut out);
                    out.push((rest[1..mid].to_owned(), link));
                    rest = &rest[mid + end + 1..];
                    continue;
                }
            }
        }
        let mut chars = rest.chars();
        plain.push(chars.next().unwrap_or_default());
        rest = chars.as_str();
    }
    flush(&mut plain, &mut out);
    out
}

/// Block body text with `#` headings and `-` bullets.
fn markdown_lite(theme: Theme, text: &str) -> Vec<Vec<(String, TuiStyle)>> {
    let mut rows = Vec::new();
    for line in text.split('\n') {
        if let Some(heading) = line.strip_prefix("# ") {
            rows.push(vec![(
                heading.to_owned(),
                theme.primary_text_style().add_modifier(Modifier::BOLD),
            )]);
        } else if let Some(heading) = line.strip_prefix("## ") {
            rows.push(vec![(
                heading.to_owned(),
                theme.primary_text_style().add_modifier(Modifier::BOLD),
            )]);
        } else if let Some(item) = line.strip_prefix("- ") {
            let mut row = vec![("- ".to_owned(), theme.muted_text_style())];
            row.extend(inline_spans(theme, item, theme.primary_text_style()));
            rows.push(row);
        } else {
            rows.push(inline_spans(theme, line, theme.primary_text_style()));
        }
    }
    rows
}

/// Render one block into rows (without the leading blank separator).
/// Control characters (including pasted escape sequences) are stripped from
/// text rows: backend content is data, never terminal instructions.
pub fn block_rows(theme: Theme, block: &Block) -> Vec<TxRow> {
    let rows = block_rows_inner(theme, block);
    rows.into_iter()
        .map(|row| match row {
            TxRow::Text(spans) => TxRow::Text(
                spans
                    .into_iter()
                    .map(|(text, style)| {
                        (
                            text.chars()
                                .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
                                .collect(),
                            style,
                        )
                    })
                    .collect(),
            ),
            other => other,
        })
        .collect()
}

/// Render one block into rows (without the leading blank separator).
fn block_rows_inner(theme: Theme, block: &Block) -> Vec<TxRow> {
    match block {
        Block::User { text } => {
            // Warp prefixes submitted input with an accent `>` marker.
            let mut rows = Vec::new();
            for (i, line) in text.split('\n').enumerate() {
                if i == 0 {
                    let mut row = vec![("> ".to_owned(), theme.input_prefix_style())];
                    row.extend(inline_spans(theme, line, theme.primary_text_style()));
                    rows.push(TxRow::Text(row));
                } else {
                    let mut row = vec![("  ".to_owned(), theme.primary_text_style())];
                    row.extend(inline_spans(theme, line, theme.primary_text_style()));
                    rows.push(TxRow::Text(row));
                }
            }
            rows
        }
        Block::Thinking { text } => {
            let mut rows = vec![text_row(
                "Thinking…",
                theme.muted_text_style().add_modifier(Modifier::ITALIC),
            )];
            for line in text.split('\n') {
                rows.push(text_row(line.to_owned(), theme.muted_text_style()));
            }
            rows
        }
        Block::Assistant { text } => markdown_lite(theme, text)
            .into_iter()
            .map(TxRow::Text)
            .collect(),
        Block::Tool { call } => {
            let (glyph, glyph_style) = match call.state {
                ToolState::Done => ("✓", theme.success_glyph_style()),
                ToolState::Failed => ("✗", theme.error_text_style()),
                ToolState::Running => ("◐", theme.attention_glyph_style()),
                ToolState::Waiting => ("○", theme.dim_text_style()),
            };
            vec![TxRow::Text(vec![
                (format!("{glyph} {} ", call.name), glyph_style),
                (
                    format!("{} · {} · {}", call.detail, call.state, call.elapsed),
                    theme.muted_text_style(),
                ),
            ])]
        }
        Block::Shell { run } => {
            let bg = theme.shell_command_background();
            let (marker_glyph, marker_style) = match run.state {
                ToolState::Done => ("!", theme.shell_command_accent_style().bg(bg)),
                ToolState::Failed => ("!", theme.error_text_style().bg(bg)),
                ToolState::Running => ("◐", theme.attention_glyph_style().bg(bg)),
                ToolState::Waiting => ("○", theme.dim_text_style().bg(bg)),
            };
            let body = theme.primary_text_style().bg(bg);
            let mut rows = vec![TxRow::Text(vec![
                (format!("{marker_glyph} "), marker_style),
                (run.command.clone(), body.add_modifier(Modifier::BOLD)),
            ])];
            for line in &run.output {
                rows.push(TxRow::Text(vec![
                    ("  ".to_owned(), body),
                    (line.clone(), body),
                ]));
            }
            rows
        }
        Block::Edits { files } => {
            let mut rows = Vec::new();
            for file in files {
                rows.push(TxRow::Text(vec![
                    ("+ ".to_owned(), theme.link_text_style()),
                    (
                        file.path.clone(),
                        theme.link_text_style().add_modifier(Modifier::BOLD),
                    ),
                    (
                        format!("  +{} −{}", file.added, file.removed),
                        theme.muted_text_style(),
                    ),
                ]));
                for (is_add, line) in &file.lines {
                    if *is_add {
                        rows.push(TxRow::Text(vec![
                            ("+ ".to_owned(), theme.diff_added_style()),
                            (line.clone(), theme.diff_added_style()),
                        ]));
                    } else {
                        rows.push(TxRow::Text(vec![
                            ("− ".to_owned(), theme.diff_removed_style()),
                            (line.clone(), theme.diff_removed_style()),
                        ]));
                    }
                }
            }
            rows
        }
        Block::Plan { title, body } => {
            let bg = theme.plan_background();
            let mut rows = vec![TxRow::Text(vec![(
                format!("Plan: {title}"),
                theme
                    .brand_accent_style()
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            )])];
            for line in body.split('\n') {
                rows.push(TxRow::Text(vec![(
                    line.to_owned(),
                    theme.muted_text_style().bg(bg),
                )]));
            }
            rows
        }
        Block::Permission { req } => vec![
            TxRow::Text(vec![
                (
                    "◉ Permission — ".to_owned(),
                    theme.attention_glyph_style().add_modifier(Modifier::BOLD),
                ),
                (
                    req.tool.clone(),
                    theme.primary_text_style().add_modifier(Modifier::BOLD),
                ),
            ]),
            TxRow::Text(vec![
                ("  ".to_owned(), theme.primary_text_style()),
                (req.summary.clone(), theme.muted_text_style()),
            ]),
            TxRow::Text(vec![(
                "  1) Yes   2) No".to_owned(),
                theme.accent_text_style(),
            )]),
        ],
        Block::Question { q } => {
            let mut rows = vec![TxRow::Text(vec![
                (
                    "? ".to_owned(),
                    theme.accent_text_style().add_modifier(Modifier::BOLD),
                ),
                (
                    q.prompt.clone(),
                    theme.primary_text_style().add_modifier(Modifier::BOLD),
                ),
            ])];
            for (i, option) in q.options.iter().enumerate() {
                rows.push(TxRow::Text(vec![(
                    format!("  {}) {option}", i + 1),
                    theme.primary_text_style(),
                )]));
            }
            rows
        }
        Block::Working { label } => vec![TxRow::Spinner {
            label: label.clone(),
        }],
        Block::Error { text } => vec![TxRow::Text(vec![(
            format!("! {text}"),
            theme.error_text_style(),
        )])],
        Block::Notice { text, link } => {
            let mut rows = vec![text_row(text.clone(), theme.dim_text_style())];
            if let Some((label, url)) = link {
                rows.push(TxRow::Link {
                    label: label.clone(),
                    url: url.clone(),
                });
            }
            rows
        }
    }
}

/// Build a finished element for a single visual (pre-wrapped) row.
/// Multi-line wrapping happens in the scroll container via [`wrap_spans`];
/// `truncate` is a safety net for wide graphemes.
pub fn row_element(theme: Theme, link: &TuiLink, row: TxRow) -> Box<dyn TuiElement> {
    match row {
        TxRow::Text(spans) => {
            if spans.iter().all(|(s, _)| s.is_empty()) {
                TuiText::new(" ").finish()
            } else {
                TuiText::from_spans(spans).truncate().finish()
            }
        }
        TxRow::Link { label, url } => {
            let action_url = url.clone();
            link.render(label, theme.link_text_style(), move |event_ctx, _| {
                event_ctx.dispatch_typed_action(SessionAction::LinkOpened(action_url.clone()));
            })
        }
        TxRow::Spinner { label } => {
            // Process-wide epoch: row elements are rebuilt every layout pass,
            // so a per-element start time would pin the spinner to frame 0.
            static EPOCH: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
            let style = theme.attention_glyph_style();
            let body = theme.muted_text_style();
            let label = label.clone();
            TuiAnimated::new(Duration::from_millis(120), move || {
                let frame = SPINNER_FRAMES[(EPOCH.elapsed().as_millis() / 120 % 10) as usize];
                TuiText::from_spans([(format!("{frame} "), style), (label.clone(), body)])
                    .truncate()
                    .finish()
            })
            .finish()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{block_rows, wrap_spans, TxRow};
    use crate::backend::{Block, ToolCall, ToolState};
    use crate::theme::Theme;
    use warpui_core::elements::tui::TuiStyle;

    #[test]
    fn wrap_breaks_long_spans() {
        let spans = vec![("hello world".to_owned(), TuiStyle::default())];
        let lines = wrap_spans(&spans, 5);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0][0].0, "hello");
    }

    #[test]
    fn user_block_has_accent_prefix() {
        let rows = block_rows(Theme, &Block::User { text: "hi".into() });
        assert_eq!(rows.len(), 1);
        match &rows[0] {
            TxRow::Text(spans) => assert_eq!(spans[0].0, "> "),
            other => panic!("expected text row, got {other:?}"),
        }
    }

    #[test]
    fn tool_row_states_render() {
        for state in [
            ToolState::Running,
            ToolState::Done,
            ToolState::Failed,
            ToolState::Waiting,
        ] {
            let rows = block_rows(
                Theme,
                &Block::Tool {
                    call: ToolCall {
                        name: "X".into(),
                        detail: "d".into(),
                        state,
                        output: Vec::new(),
                        elapsed: "0s".into(),
                    },
                },
            );
            assert_eq!(rows.len(), 1);
        }
    }

    #[test]
    fn wrap_counts_wide_graphemes() {
        use warpui_core::elements::tui::text_width;
        // CJK is 2 columns; emoji presentation varies but never 0-width here.
        assert_eq!(usize::from(text_width("中")), 2);
        let spans = vec![("ab中de".to_owned(), TuiStyle::default())];
        let lines = wrap_spans(&spans, 4);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0][0].0, "ab中");
    }

    #[test]
    fn long_unbroken_token_breaks_mid_word() {
        // A token wider than the viewport is split across visual rows so no
        // single string can destroy the layout; truncation stays a backstop.
        use warpui_core::elements::tui::text_width;
        let token = "x".repeat(300);
        let spans = vec![(token, TuiStyle::default())];
        let lines = wrap_spans(&spans, 80);
        assert_eq!(lines.len(), 4);
        for line in &lines {
            let width: usize = line
                .iter()
                .map(|(text, _)| usize::from(text_width(text)))
                .sum();
            assert!(width <= 80);
        }
    }

    #[test]
    fn ansi_bytes_pass_through_uninterpreted() {
        // Backend text is never parsed as terminal escapes: control chars
        // render literally-or-dropped by the cell grid, never executed.
        let evil = "\x1b[2J\x1b[1;1Howned".to_owned();
        let rows = block_rows(Theme, &Block::Assistant { text: evil });
        let flat: String = rows
            .iter()
            .filter_map(|row| match row {
                TxRow::Text(spans) => Some(spans),
                _ => None,
            })
            .flatten()
            .map(|(text, _)| text.clone())
            .collect();
        assert!(flat.contains("owned"));
        assert!(!flat.contains('\x1b'));
    }

    #[test]
    fn assistant_renders_all_block_shapes() {
        let blocks = vec![
            Block::Thinking { text: "t".into() },
            Block::Assistant {
                text: "# H\n- a\n`c` **b** [l](u)".into(),
            },
            Block::Shell {
                run: crate::backend::ShellRun {
                    command: "ls".into(),
                    output: vec!["a".into()],
                    state: ToolState::Done,
                },
            },
            Block::Edits {
                files: vec![crate::backend::FileDiff {
                    path: "f".into(),
                    added: 1,
                    removed: 1,
                    lines: vec![(true, "+a".into()), (false, "-b".into())],
                }],
            },
            Block::Plan {
                title: "t".into(),
                body: "b".into(),
            },
            Block::Permission {
                req: crate::backend::PermissionRequest {
                    tool: "t".into(),
                    summary: "s".into(),
                },
            },
            Block::Question {
                q: crate::backend::Question {
                    prompt: "p".into(),
                    options: vec!["o".into()],
                },
            },
            Block::Working { label: "w".into() },
            Block::Error { text: "e".into() },
            Block::Notice {
                text: "n".into(),
                link: None,
            },
        ];
        for block in &blocks {
            assert!(!block_rows(Theme, block).is_empty());
        }
    }
}
