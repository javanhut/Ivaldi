//! Modal text input overlay dialog.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::input::TextInput;
use crate::tui::theme::Theme;

/// A modal dialog with a title and text input.
pub struct Dialog {
    pub title: String,
    pub input: TextInput,
    pub visible: bool,
}

impl Dialog {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            input: TextInput::new(),
            visible: false,
        }
    }

    pub fn show(&mut self, title: impl Into<String>) {
        self.title = title.into();
        self.input.clear();
        self.visible = true;
    }

    pub fn show_with_value(&mut self, title: impl Into<String>, value: String) {
        self.title = title.into();
        self.input = TextInput::with_value(value);
        self.visible = true;
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.input.clear();
    }

    pub fn value(&self) -> &str {
        &self.input.value
    }

    pub fn render(&self, frame: &mut Frame, screen: Rect, theme: &Theme) {
        if !self.visible {
            return;
        }

        // Center a dialog box
        let width = (screen.width / 2).max(30).min(screen.width.saturating_sub(4));
        let height = 5;
        let x = (screen.width.saturating_sub(width)) / 2;
        let y = (screen.height.saturating_sub(height)) / 2;

        let area = Rect::new(x, y, width, height);

        // Clear background
        frame.render_widget(Clear, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.brand)
            .title(Span::styled(
                format!(" {} ", self.title),
                theme.title,
            ));

        frame.render_widget(block, area);

        // Input area inside the block
        let input_area = Rect::new(
            area.x + 2,
            area.y + 2,
            area.width.saturating_sub(4),
            1,
        );

        self.input.render(frame, input_area, Style::default().fg(Color::White));

        // Hint at bottom
        let hint_area = Rect::new(
            area.x + 2,
            area.y + 3,
            area.width.saturating_sub(4),
            1,
        );
        let hint = Paragraph::new(Span::styled(
            "Enter: submit  Esc: cancel",
            theme.dim,
        ));
        frame.render_widget(hint, hint_area);
    }
}
