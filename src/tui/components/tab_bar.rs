//! Top tab navigation bar.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::tui::theme::Theme;
use crate::tui::types::TabId;

/// Renders the tab bar at the top of the screen.
pub struct TabBar;

impl TabBar {
    pub fn render(frame: &mut Frame, area: Rect, active: TabId, theme: &Theme) {
        let mut spans = Vec::new();

        for (i, tab) in TabId::ALL.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }

            let label = format!(" {} {} ", i + 1, tab.label());
            let style = if *tab == active {
                theme.tab_active
            } else {
                theme.tab_inactive
            };
            spans.push(Span::styled(label, style));
        }

        let line = Line::from(spans);
        let para = Paragraph::new(line);
        frame.render_widget(para, area);
    }
}
