//! Shelves tab — view auto-shelved working-tree state across timelines.
//!
//! Each shelf is keyed by the timeline name it was captured on. The list
//! shows a one-line summary per shelf (modified / untracked / deleted /
//! staged counts) and Enter expands the selected entry to show every
//! affected path. `d` removes the selected shelf.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::shelf::{Shelf, ShelfManager, WorkspaceChange};
use crate::tui::theme::Theme;
use crate::tui::types::{Action, AppContext};
use crate::tui::views::TabView;

/// Summary numbers cached for each shelf so we can render the list quickly.
struct ShelfSummary {
    timeline: String,
    created_at: i64,
    modified: usize,
    untracked: usize,
    deleted: usize,
    staged: usize,
}

impl ShelfSummary {
    fn from(shelf: &Shelf) -> Self {
        let mut modified = 0;
        let mut untracked = 0;
        let mut deleted = 0;
        for c in &shelf.workspace_changes {
            match c {
                WorkspaceChange::Modified { .. } => modified += 1,
                WorkspaceChange::Untracked { .. } => untracked += 1,
                WorkspaceChange::Deleted { .. } => deleted += 1,
            }
        }
        Self {
            timeline: shelf.timeline.clone(),
            created_at: shelf.created_at,
            modified,
            untracked,
            deleted,
            staged: shelf.staged_files.len(),
        }
    }

    fn one_line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.modified > 0 {
            parts.push(format!("{}M", self.modified));
        }
        if self.untracked > 0 {
            parts.push(format!("{}U", self.untracked));
        }
        if self.deleted > 0 {
            parts.push(format!("{}D", self.deleted));
        }
        if self.staged > 0 {
            parts.push(format!("{}S", self.staged));
        }
        let counts = if parts.is_empty() {
            "(empty)".into()
        } else {
            parts.join(" ")
        };
        format!(
            "{:<24} {:>10}  {}",
            self.timeline,
            relative_time(self.created_at),
            counts
        )
    }
}

fn relative_time(unix: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let diff = now - unix;
    if diff < 60 {
        "just now".into()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

pub struct ShelvesView {
    summaries: Vec<ShelfSummary>,
    /// Lazily-loaded full shelf for the cursor entry; populated when the user
    /// presses Enter to expand. Cleared when the cursor moves so the detail
    /// panel only ever shows a shelf the user explicitly opened.
    expanded: Option<Shelf>,
    cursor: usize,
    message: Option<(String, bool)>,
}

impl Default for ShelvesView {
    fn default() -> Self {
        Self::new()
    }
}

impl ShelvesView {
    pub fn new() -> Self {
        Self {
            summaries: Vec::new(),
            expanded: None,
            cursor: 0,
            message: None,
        }
    }

    fn move_to(&mut self, idx: usize) {
        self.cursor = idx.min(self.summaries.len().saturating_sub(1));
        self.expanded = None;
    }

    fn expand_current(&mut self, ctx: &AppContext) -> Action {
        if let Some(summary) = self.summaries.get(self.cursor) {
            let mgr = ShelfManager::new(&ctx.ivaldi_dir);
            match mgr.load_shelf(&summary.timeline) {
                Ok(Some(shelf)) => {
                    self.expanded = Some(shelf);
                    Action::Consumed
                }
                Ok(None) => Action::Error("Shelf disappeared".into()),
                Err(e) => Action::Error(format!("Failed to load shelf: {}", e)),
            }
        } else {
            Action::Consumed
        }
    }

    fn drop_current(&mut self, ctx: &AppContext) -> Action {
        let timeline = match self.summaries.get(self.cursor) {
            Some(s) => s.timeline.clone(),
            None => return Action::Consumed,
        };
        let mgr = ShelfManager::new(&ctx.ivaldi_dir);
        match mgr.remove_shelf(&timeline) {
            Ok(()) => {
                self.message = Some((format!("Dropped shelf for '{}'", timeline), false));
                self.expanded = None;
                Action::Refresh
            }
            Err(e) => Action::Error(format!("Failed to drop shelf: {}", e)),
        }
    }
}

impl TabView for ShelvesView {
    fn handle_event(&mut self, event: &KeyEvent, ctx: &mut AppContext) -> Action {
        match event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.summaries.is_empty() && self.cursor + 1 < self.summaries.len() {
                    self.move_to(self.cursor + 1);
                }
                Action::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.cursor > 0 {
                    self.move_to(self.cursor - 1);
                }
                Action::Consumed
            }
            KeyCode::Char('g') => {
                self.move_to(0);
                Action::Consumed
            }
            KeyCode::Char('G') => {
                if !self.summaries.is_empty() {
                    self.move_to(self.summaries.len() - 1);
                }
                Action::Consumed
            }
            KeyCode::Enter => self.expand_current(ctx),
            KeyCode::Char('d') => self.drop_current(ctx),
            KeyCode::Char('r') => Action::Refresh,
            _ => Action::None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.summaries.is_empty() {
            let msg = Paragraph::new(Span::styled(
                "No shelves. They are created automatically when you switch timelines with dirty changes.",
                theme.dim,
            ));
            frame.render_widget(msg, area);
            return;
        }

        // Layout: left half = list, right half = detail (when expanded).
        let split_x = if self.expanded.is_some() {
            area.width / 2
        } else {
            area.width
        };

        let list_area = Rect {
            x: area.x,
            y: area.y,
            width: split_x,
            height: area.height.saturating_sub(1),
        };

        let items: Vec<ListItem> = self
            .summaries
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let marker = if i == self.cursor { ">" } else { " " };
                let line = format!("{} {}", marker, s.one_line());
                let style = if i == self.cursor {
                    theme.cursor
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Span::styled(line, style))
            })
            .collect();

        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            format!(" Shelves ({}) ", self.summaries.len()),
            theme.title,
        ));
        frame.render_widget(List::new(items).block(block), list_area);

        if let Some(shelf) = &self.expanded {
            let detail_area = Rect {
                x: area.x + split_x,
                y: area.y,
                width: area.width - split_x,
                height: area.height.saturating_sub(1),
            };
            let mut lines: Vec<Line> = Vec::new();
            lines.push(Line::from(Span::styled(
                format!("Shelf for '{}'", shelf.timeline),
                theme.title,
            )));
            lines.push(Line::from(""));
            for path in shelf.staged_files.keys() {
                lines.push(Line::from(vec![
                    Span::styled(" S ", theme.help_key),
                    Span::raw(path.clone()),
                ]));
            }
            for change in &shelf.workspace_changes {
                let (tag, path) = match change {
                    WorkspaceChange::Modified { path, .. } => ("M", path),
                    WorkspaceChange::Untracked { path, .. } => ("U", path),
                    WorkspaceChange::Deleted { path } => ("D", path),
                };
                lines.push(Line::from(vec![
                    Span::styled(format!(" {} ", tag), theme.help_key),
                    Span::raw(path.clone()),
                ]));
            }

            let detail_block = Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(" Detail ", theme.title));
            let para = Paragraph::new(lines).block(detail_block);
            frame.render_widget(para, detail_area);
        }

        // Help line
        if area.height > 1 {
            let help_area = Rect {
                x: area.x,
                y: area.y + area.height - 1,
                width: area.width,
                height: 1,
            };
            let msg = match &self.message {
                Some((m, true)) => Span::styled(format!(" {}", m), theme.error),
                Some((m, false)) => Span::styled(format!(" {}", m), theme.success),
                None => Span::styled(" Enter:expand  d:drop  r:refresh", theme.dim),
            };
            frame.render_widget(Paragraph::new(msg), help_area);
        }
    }

    fn load_data(&mut self, ctx: &AppContext) {
        let mgr = ShelfManager::new(&ctx.ivaldi_dir);
        let names = mgr.list_shelves().unwrap_or_default();

        let mut summaries: Vec<ShelfSummary> = Vec::new();
        for name in names {
            if let Ok(Some(shelf)) = mgr.load_shelf(&name) {
                summaries.push(ShelfSummary::from(&shelf));
            }
        }
        // Most recently modified first.
        summaries.sort_by_key(|s| std::cmp::Reverse(s.created_at));

        self.summaries = summaries;
        if self.cursor >= self.summaries.len() {
            self.cursor = self.summaries.len().saturating_sub(1);
        }
        // Drop any expanded view that may now point at a dead entry.
        self.expanded = None;
    }

    fn short_help(&self) -> &str {
        "Enter:expand d:drop r:refresh"
    }

    fn has_active_input(&self) -> bool {
        false
    }
}
