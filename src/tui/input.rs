//! Single-line text input widget for the TUI dashboard.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// A single-line text editor with cursor.
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
}

impl TextInput {
    pub fn new() -> Self {
        Self {
            value: String::new(),
            cursor: 0,
        }
    }

    pub fn with_value(value: String) -> Self {
        let cursor = value.len();
        Self { value, cursor }
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// Handle a key event. Returns true if the event was consumed.
    pub fn handle_key(&mut self, key: &KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c) => {
                self.value.insert(self.cursor, c);
                self.cursor += 1;
                true
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.value.remove(self.cursor);
                }
                true
            }
            KeyCode::Delete => {
                if self.cursor < self.value.len() {
                    self.value.remove(self.cursor);
                }
                true
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                true
            }
            KeyCode::Right => {
                if self.cursor < self.value.len() {
                    self.cursor += 1;
                }
                true
            }
            KeyCode::Home => {
                self.cursor = 0;
                true
            }
            KeyCode::End => {
                self.cursor = self.value.len();
                true
            }
            _ => false,
        }
    }

    /// Render the text input into the given area.
    pub fn render(&self, frame: &mut Frame, area: Rect, style: Style) {
        let display = if self.value.is_empty() {
            String::new()
        } else {
            self.value.clone()
        };

        let para = Paragraph::new(display).style(style);
        frame.render_widget(para, area);

        // Place cursor
        if area.width > 0 {
            let cx = area.x + self.cursor as u16;
            if cx < area.x + area.width {
                frame.set_cursor_position((cx, area.y));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    #[test]
    fn insert_chars() {
        let mut input = TextInput::new();
        input.handle_key(&key(KeyCode::Char('h')));
        input.handle_key(&key(KeyCode::Char('i')));
        assert_eq!(input.value, "hi");
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn backspace_at_start() {
        let mut input = TextInput::new();
        input.handle_key(&key(KeyCode::Backspace));
        assert_eq!(input.value, "");
        assert_eq!(input.cursor, 0);
    }

    #[test]
    fn backspace_removes_char() {
        let mut input = TextInput::with_value("abc".into());
        input.handle_key(&key(KeyCode::Backspace));
        assert_eq!(input.value, "ab");
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn delete_at_cursor() {
        let mut input = TextInput::with_value("abc".into());
        input.cursor = 1;
        input.handle_key(&key(KeyCode::Delete));
        assert_eq!(input.value, "ac");
        assert_eq!(input.cursor, 1);
    }

    #[test]
    fn left_right_navigation() {
        let mut input = TextInput::with_value("abc".into());
        assert_eq!(input.cursor, 3);
        input.handle_key(&key(KeyCode::Left));
        assert_eq!(input.cursor, 2);
        input.handle_key(&key(KeyCode::Left));
        assert_eq!(input.cursor, 1);
        input.handle_key(&key(KeyCode::Right));
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn home_end() {
        let mut input = TextInput::with_value("hello".into());
        input.handle_key(&key(KeyCode::Home));
        assert_eq!(input.cursor, 0);
        input.handle_key(&key(KeyCode::End));
        assert_eq!(input.cursor, 5);
    }

    #[test]
    fn cursor_bounds() {
        let mut input = TextInput::new();
        input.handle_key(&key(KeyCode::Left));
        assert_eq!(input.cursor, 0);
        input.handle_key(&key(KeyCode::Right));
        assert_eq!(input.cursor, 0);
    }

    #[test]
    fn insert_in_middle() {
        let mut input = TextInput::with_value("ac".into());
        input.cursor = 1;
        input.handle_key(&key(KeyCode::Char('b')));
        assert_eq!(input.value, "abc");
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn clear() {
        let mut input = TextInput::with_value("hello".into());
        input.clear();
        assert_eq!(input.value, "");
        assert_eq!(input.cursor, 0);
    }
}
