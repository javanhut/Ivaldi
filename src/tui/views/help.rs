//! Help overlay — shows global keybindings.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::theme::Theme;

const HELP_ENTRIES: &[(&str, &str)] = &[
    ("1-6", "Jump to tab"),
    ("Tab", "Next tab"),
    ("Shift+Tab", "Previous tab"),
    ("?", "Toggle this help"),
    ("Esc", "Close dialog/help"),
    ("q / Ctrl+C", "Quit"),
    ("", ""),
    ("j / Down", "Move down"),
    ("k / Up", "Move up"),
    ("g", "Go to top"),
    ("G", "Go to bottom"),
    ("r", "Refresh data"),
];

/// Render the help overlay centered on screen.
pub fn render_help(frame: &mut Frame, theme: &Theme) {
    let screen = frame.area();
    let width = 40u16.min(screen.width.saturating_sub(4));
    let height = (HELP_ENTRIES.len() as u16 + 2).min(screen.height.saturating_sub(2));
    let x = (screen.width.saturating_sub(width)) / 2;
    let y = (screen.height.saturating_sub(height)) / 2;
    let area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.brand)
        .title(Span::styled(" Help ", theme.title));

    let lines: Vec<Line> = HELP_ENTRIES
        .iter()
        .map(|(key, desc)| {
            if key.is_empty() {
                Line::from("")
            } else {
                Line::from(vec![
                    Span::styled(format!("{:>14}  ", key), theme.help_key),
                    Span::styled(*desc, theme.help_desc),
                ])
            }
        })
        .collect();

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}
