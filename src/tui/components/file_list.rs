//! Scrollable file list with cursor and toggle selection.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::tui::theme::Theme;
use crate::workspace::FileState;

/// An item in the file list.
#[derive(Debug, Clone)]
pub struct FileItem {
    pub path: String,
    pub state: FileState,
    pub selected: bool,
}

impl FileItem {
    pub fn status_char(&self) -> &'static str {
        match self.state {
            FileState::Staged => "S",
            FileState::Modified => "M",
            FileState::Untracked => "?",
            FileState::Deleted => "D",
            FileState::Unmodified => " ",
        }
    }
}

/// A scrollable file list component.
pub struct FileListWidget {
    pub items: Vec<FileItem>,
    pub cursor: usize,
    pub offset: usize,
}

impl FileListWidget {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            cursor: 0,
            offset: 0,
        }
    }

    pub fn set_items(&mut self, items: Vec<FileItem>) {
        self.items = items;
        self.cursor = 0;
        self.offset = 0;
    }

    pub fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if !self.items.is_empty() && self.cursor < self.items.len() - 1 {
            self.cursor += 1;
        }
    }

    pub fn move_top(&mut self) {
        self.cursor = 0;
    }

    pub fn move_bottom(&mut self) {
        if !self.items.is_empty() {
            self.cursor = self.items.len() - 1;
        }
    }

    pub fn toggle_current(&mut self) {
        if let Some(item) = self.items.get_mut(self.cursor) {
            item.selected = !item.selected;
        }
    }

    pub fn current_path(&self) -> Option<&str> {
        self.items.get(self.cursor).map(|i| i.path.as_str())
    }

    pub fn current_item(&self) -> Option<&FileItem> {
        self.items.get(self.cursor)
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, title: &str, theme: &Theme) {
        let visible = area.height.saturating_sub(2) as usize; // borders
        // Adjust offset for scrolling
        let offset = if self.cursor < self.offset {
            self.cursor
        } else if self.cursor >= self.offset + visible {
            self.cursor.saturating_sub(visible.saturating_sub(1))
        } else {
            self.offset
        };

        let items: Vec<ListItem> = self
            .items
            .iter()
            .enumerate()
            .skip(offset)
            .take(visible)
            .map(|(i, item)| {
                let marker = if i == self.cursor { ">" } else { " " };
                let sel = if item.selected { "*" } else { " " };
                let status = item.status_char();
                let text = format!("{}{} {} {}", marker, sel, status, item.path);

                let style = if i == self.cursor {
                    theme.cursor
                } else {
                    match item.state {
                        FileState::Staged => theme.staged,
                        FileState::Modified => theme.modified,
                        FileState::Deleted => theme.deleted,
                        FileState::Untracked => theme.untracked,
                        FileState::Unmodified => theme.dim,
                    }
                };

                ListItem::new(Span::styled(text, style))
            })
            .collect();

        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            format!(" {} ({}) ", title, self.items.len()),
            theme.title,
        ));

        let list = List::new(items).block(block);
        frame.render_widget(list, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_items(n: usize) -> Vec<FileItem> {
        (0..n)
            .map(|i| FileItem {
                path: format!("file{}.rs", i),
                state: FileState::Modified,
                selected: false,
            })
            .collect()
    }

    #[test]
    fn navigation_bounds() {
        let mut w = FileListWidget::new();
        w.set_items(make_items(5));

        w.move_up(); // already at 0
        assert_eq!(w.cursor, 0);

        w.move_bottom();
        assert_eq!(w.cursor, 4);

        w.move_down(); // already at end
        assert_eq!(w.cursor, 4);

        w.move_top();
        assert_eq!(w.cursor, 0);
    }

    #[test]
    fn toggle_selection() {
        let mut w = FileListWidget::new();
        w.set_items(make_items(3));
        assert!(!w.items[0].selected);
        w.toggle_current();
        assert!(w.items[0].selected);
        w.toggle_current();
        assert!(!w.items[0].selected);
    }

    #[test]
    fn empty_list_operations() {
        let mut w = FileListWidget::new();
        w.move_up();
        w.move_down();
        w.move_top();
        w.move_bottom();
        w.toggle_current();
        assert!(w.current_path().is_none());
    }
}
