//! Bottom info bar showing timeline, seal, and file counts.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::tui::theme::Theme;
use crate::tui::types::StatusData;

/// Renders the status bar at the bottom of the screen.
pub struct StatusBar;

impl StatusBar {
    pub fn render(
        frame: &mut Frame,
        area: Rect,
        data: &StatusData,
        message: Option<&(String, bool)>,
        theme: &Theme,
    ) {
        // First line: status message or help hint
        if area.height >= 2 {
            let msg_area = Rect { height: 1, ..area };
            if let Some((msg, is_error)) = message {
                let style = if *is_error {
                    theme.error
                } else {
                    theme.success
                };
                let para = Paragraph::new(Span::styled(msg.as_str(), style));
                frame.render_widget(para, msg_area);
            }
        }

        // Last line: timeline / seal / file counts
        let bar_y = if area.height >= 2 {
            area.y + area.height - 1
        } else {
            area.y
        };
        let bar_area = Rect {
            x: area.x,
            y: bar_y,
            width: area.width,
            height: 1,
        };

        let mut spans = Vec::new();
        spans.push(Span::styled(
            format!(" {} ", data.timeline),
            theme.tab_active,
        ));
        spans.push(Span::raw(" "));

        if !data.seal_name.is_empty() {
            spans.push(Span::styled(&data.seal_name, theme.dim));
            spans.push(Span::raw(" "));
        }

        if data.staged > 0 {
            spans.push(Span::styled(format!("{}S ", data.staged), theme.staged));
        }
        if data.modified > 0 {
            spans.push(Span::styled(format!("{}M ", data.modified), theme.modified));
        }
        if data.untracked > 0 {
            spans.push(Span::styled(
                format!("{}? ", data.untracked),
                theme.untracked,
            ));
        }
        if data.deleted > 0 {
            spans.push(Span::styled(format!("{}D ", data.deleted), theme.deleted));
        }

        let line = Line::from(spans);
        let para = Paragraph::new(line).style(theme.status_bar);
        frame.render_widget(para, bar_area);
    }
}
