//! Log tab — commit history viewer.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::repo::HistoryEntry;
use crate::tui::theme::Theme;
use crate::tui::types::{Action, AppContext};
use crate::tui::views::TabView;

pub struct LogView {
    entries: Vec<HistoryEntry>,
    cursor: usize,
    offset: usize,
    oneline: bool,
    all_timelines: bool,
}

impl LogView {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            offset: 0,
            oneline: false,
            all_timelines: false,
        }
    }

    fn format_time(unix: i64) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let diff = now - unix;

        if diff < 60 {
            "just now".to_string()
        } else if diff < 3600 {
            format!("{}m ago", diff / 60)
        } else if diff < 86400 {
            format!("{}h ago", diff / 3600)
        } else {
            format!("{}d ago", diff / 86400)
        }
    }
}

impl TabView for LogView {
    fn handle_event(&mut self, event: &KeyEvent, _ctx: &mut AppContext) -> Action {
        match event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.entries.is_empty() && self.cursor < self.entries.len() - 1 {
                    self.cursor += 1;
                }
                Action::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                Action::Consumed
            }
            KeyCode::Char('g') => {
                self.cursor = 0;
                Action::Consumed
            }
            KeyCode::Char('G') => {
                if !self.entries.is_empty() {
                    self.cursor = self.entries.len() - 1;
                }
                Action::Consumed
            }
            KeyCode::Char('o') => {
                self.oneline = !self.oneline;
                Action::Consumed
            }
            KeyCode::Char('t') => {
                self.all_timelines = !self.all_timelines;
                Action::Refresh
            }
            KeyCode::Char('r') => Action::Refresh,
            _ => Action::None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.entries.is_empty() {
            let msg = Paragraph::new(Span::styled("No history yet", theme.dim));
            frame.render_widget(msg, area);
            return;
        }

        let inner_height = area.height.saturating_sub(2) as usize;

        // Adjust offset for cursor visibility
        let offset = if self.cursor < self.offset {
            self.cursor
        } else if self.oneline {
            if self.cursor >= self.offset + inner_height {
                self.cursor.saturating_sub(inner_height.saturating_sub(1))
            } else {
                self.offset
            }
        } else {
            // In full mode each entry takes 3 lines
            let entries_per_page = inner_height / 3;
            if self.cursor >= self.offset + entries_per_page {
                self.cursor.saturating_sub(entries_per_page.saturating_sub(1))
            } else {
                self.offset
            }
        };

        if self.oneline {
            let items: Vec<ListItem> = self
                .entries
                .iter()
                .enumerate()
                .skip(offset)
                .take(inner_height)
                .map(|(i, entry)| {
                    let marker = if i == self.cursor { ">" } else { " " };
                    let merge_flag = if entry.is_merge { " M" } else { "" };
                    let text = format!(
                        "{} {} {} {}{}",
                        marker,
                        &entry.short_hash,
                        entry.seal_name,
                        entry.message,
                        merge_flag,
                    );
                    let style = if i == self.cursor {
                        theme.cursor
                    } else {
                        Style::default().fg(Color::White)
                    };
                    ListItem::new(Span::styled(text, style))
                })
                .collect();

            let mode = if self.all_timelines { "all" } else { "current" };
            let block = Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    format!(" Log ({}) [{}/{}] ", mode, self.cursor + 1, self.entries.len()),
                    theme.title,
                ));

            let list = List::new(items).block(block);
            frame.render_widget(list, area);
        } else {
            // Full format: 3 lines per entry
            let entries_per_page = inner_height / 3;
            let mut lines: Vec<Line> = Vec::new();

            for (i, entry) in self
                .entries
                .iter()
                .enumerate()
                .skip(offset)
                .take(entries_per_page)
            {
                let is_current = i == self.cursor;
                let marker = if is_current { ">" } else { " " };
                let merge_flag = if entry.is_merge { " [merge]" } else { "" };

                // Line 1: seal name + hash
                let name_style = if is_current { theme.cursor } else { theme.brand };
                lines.push(Line::from(vec![
                    Span::styled(format!("{} ", marker), name_style),
                    Span::styled(&entry.seal_name, name_style),
                    Span::styled(format!("  {}{}", entry.short_hash, merge_flag), theme.dim),
                ]));

                // Line 2: message
                let msg_style = if is_current {
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                lines.push(Line::from(Span::styled(
                    format!("  {}", entry.message),
                    msg_style,
                )));

                // Line 3: author + time
                let time_str = Self::format_time(entry.time_unix);
                lines.push(Line::from(Span::styled(
                    format!("  {} - {}", entry.author, time_str),
                    theme.dim,
                )));
            }

            let mode = if self.all_timelines { "all" } else { "current" };
            let block = Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(
                    format!(" Log ({}) [{}/{}] ", mode, self.cursor + 1, self.entries.len()),
                    theme.title,
                ));

            let para = Paragraph::new(lines).block(block);
            frame.render_widget(para, area);
        }
    }

    fn load_data(&mut self, ctx: &AppContext) {
        let timeline = ctx.repo.current_timeline().unwrap_or_default();

        if self.all_timelines {
            // Collect from all timelines, deduplicate by index
            let mut all_entries = Vec::new();
            let mut seen = std::collections::HashSet::new();
            if let Ok(timelines) = ctx.repo.list_timelines() {
                for (tl_name, _) in &timelines {
                    if let Ok(entries) = ctx.repo.walk_history(tl_name) {
                        for entry in entries {
                            if seen.insert(entry.index) {
                                all_entries.push(entry);
                            }
                        }
                    }
                }
            }
            all_entries.sort_by(|a, b| b.time_unix.cmp(&a.time_unix));
            self.entries = all_entries;
        } else {
            self.entries = ctx.repo.walk_history(&timeline).unwrap_or_default();
        }

        // Reset cursor if out of bounds
        if self.cursor >= self.entries.len() && !self.entries.is_empty() {
            self.cursor = self.entries.len() - 1;
        }
    }

    fn short_help(&self) -> &str {
        "j/k:navigate o:oneline t:all-timelines"
    }

    fn has_active_input(&self) -> bool {
        false
    }
}
