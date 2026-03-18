//! Timeline management tab — create, switch, delete, rename timelines.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::tui::components::dialog::Dialog;
use crate::tui::theme::Theme;
use crate::tui::types::{Action, AppContext};
use crate::tui::views::TabView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogMode {
    Create,
    Rename,
}

pub struct TimelineView {
    timelines: Vec<(String, u64, bool)>, // name, head_idx, is_current
    cursor: usize,
    dialog: Dialog,
    dialog_mode: Option<DialogMode>,
    confirm_delete: Option<String>,
}

impl TimelineView {
    pub fn new() -> Self {
        Self {
            timelines: Vec::new(),
            cursor: 0,
            dialog: Dialog::new(""),
            dialog_mode: None,
            confirm_delete: None,
        }
    }
}

impl TabView for TimelineView {
    fn handle_event(&mut self, event: &KeyEvent, ctx: &mut AppContext) -> Action {
        // Delete confirmation mode
        if let Some(ref name) = self.confirm_delete.clone() {
            match event.code {
                KeyCode::Char('y') => {
                    let result = match ctx.repo.remove_timeline(name) {
                        Ok(()) => Action::Refresh,
                        Err(e) => Action::Error(format!("Delete failed: {}", e)),
                    };
                    self.confirm_delete = None;
                    return result;
                }
                _ => {
                    self.confirm_delete = None;
                    return Action::Consumed;
                }
            }
        }

        // Dialog mode
        if self.dialog.visible {
            match event.code {
                KeyCode::Enter => {
                    let value = self.dialog.value().to_string();
                    let mode = self.dialog_mode;
                    self.dialog.hide();
                    self.dialog_mode = None;

                    if value.trim().is_empty() {
                        return Action::Consumed;
                    }

                    return match mode {
                        Some(DialogMode::Create) => {
                            match ctx.repo.create_timeline(&value, None) {
                                Ok(()) => {
                                    let _ = ctx.repo.switch_timeline(&value);
                                    Action::Refresh
                                }
                                Err(e) => Action::Error(format!("Create failed: {}", e)),
                            }
                        }
                        Some(DialogMode::Rename) => {
                            if let Some((old_name, _, _)) = self.timelines.get(self.cursor) {
                                match ctx.repo.rename_timeline(old_name, &value) {
                                    Ok(()) => Action::Refresh,
                                    Err(e) => Action::Error(format!("Rename failed: {}", e)),
                                }
                            } else {
                                Action::Consumed
                            }
                        }
                        None => Action::Consumed,
                    };
                }
                KeyCode::Esc => {
                    self.dialog.hide();
                    self.dialog_mode = None;
                    return Action::Consumed;
                }
                _ => {
                    self.dialog.input.handle_key(event);
                    return Action::Consumed;
                }
            }
        }

        match event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.timelines.is_empty() && self.cursor < self.timelines.len() - 1 {
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
            KeyCode::Enter => {
                if let Some((name, _, is_current)) = self.timelines.get(self.cursor) {
                    if !is_current {
                        let name = name.clone();
                        match ctx.repo.switch_timeline(&name) {
                            Ok(()) => Action::Refresh,
                            Err(e) => Action::Error(format!("Switch failed: {}", e)),
                        }
                    } else {
                        Action::Consumed
                    }
                } else {
                    Action::Consumed
                }
            }
            KeyCode::Char('c') => {
                self.dialog_mode = Some(DialogMode::Create);
                self.dialog.show("New Timeline Name");
                Action::Consumed
            }
            KeyCode::Char('d') => {
                if let Some((name, _, is_current)) = self.timelines.get(self.cursor) {
                    if *is_current {
                        Action::Error("Cannot delete current timeline".into())
                    } else {
                        self.confirm_delete = Some(name.clone());
                        Action::Consumed
                    }
                } else {
                    Action::Consumed
                }
            }
            KeyCode::Char('R') => {
                if let Some((name, _, _)) = self.timelines.get(self.cursor) {
                    self.dialog_mode = Some(DialogMode::Rename);
                    self.dialog.show_with_value("Rename Timeline", name.clone());
                }
                Action::Consumed
            }
            KeyCode::Char('r') => Action::Refresh,
            _ => Action::None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.timelines.is_empty() {
            let msg = Paragraph::new(Span::styled("No timelines", theme.dim));
            frame.render_widget(msg, area);
            return;
        }

        let inner_height = area.height.saturating_sub(2) as usize;

        let items: Vec<ListItem> = self
            .timelines
            .iter()
            .enumerate()
            .take(inner_height)
            .map(|(i, (name, head_idx, is_current))| {
                let marker = if i == self.cursor { ">" } else { " " };
                let current_flag = if *is_current { " *" } else { "" };
                let text = format!("{} {}{} (head: {})", marker, name, current_flag, head_idx);

                let style = if i == self.cursor {
                    theme.cursor
                } else if *is_current {
                    theme.brand
                } else {
                    Style::default().fg(Color::White)
                };

                ListItem::new(Span::styled(text, style))
            })
            .collect();

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                format!(" Timelines ({}) ", self.timelines.len()),
                theme.title,
            ));

        let list = List::new(items).block(block);
        frame.render_widget(list, area);

        // Delete confirmation overlay
        if let Some(ref name) = self.confirm_delete {
            let msg_area = Rect {
                x: area.x + 2,
                y: area.y + area.height.saturating_sub(2),
                width: area.width.saturating_sub(4),
                height: 1,
            };
            let msg = Paragraph::new(Span::styled(
                format!("Delete '{}'? y:yes any:cancel", name),
                theme.warning,
            ));
            frame.render_widget(msg, msg_area);
        }

        // Help
        if area.height > 2 && self.confirm_delete.is_none() {
            let help_area = Rect {
                x: area.x,
                y: area.y + area.height - 1,
                width: area.width,
                height: 1,
            };
            let help = Paragraph::new(Span::styled(
                " Enter:switch c:create d:delete R:rename r:refresh",
                theme.dim,
            ));
            frame.render_widget(help, help_area);
        }

        // Dialog overlay
        self.dialog.render(frame, frame.area(), theme);
    }

    fn load_data(&mut self, ctx: &AppContext) {
        let current = ctx.repo.current_timeline().unwrap_or_default();
        self.timelines = ctx
            .repo
            .list_timelines()
            .unwrap_or_default()
            .into_iter()
            .map(|(name, head)| {
                let is_current = name == current;
                (name, head, is_current)
            })
            .collect();

        // Sort: current first, then alphabetical
        self.timelines.sort_by(|a, b| {
            b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0))
        });

        if self.cursor >= self.timelines.len() && !self.timelines.is_empty() {
            self.cursor = self.timelines.len() - 1;
        }
    }

    fn short_help(&self) -> &str {
        "Enter:switch c:create d:delete R:rename"
    }

    fn has_active_input(&self) -> bool {
        self.dialog.visible || self.confirm_delete.is_some()
    }
}
