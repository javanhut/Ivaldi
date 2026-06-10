//! Review tab — local code review management.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::review::{self, Review, ReviewFilter, ReviewStatus};
use crate::tui::components::dialog::Dialog;
use crate::tui::theme::Theme;
use crate::tui::types::{Action, AppContext};
use crate::tui::views::TabView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    List,
    Detail,
    Diff,
}

pub struct ReviewView {
    mode: Mode,
    reviews: Vec<Review>,
    cursor: usize,
    detail_scroll: usize,
    diff_lines: Vec<String>,
    diff_offset: usize,
    selected_review: Option<Review>,
    comment_dialog: Dialog,
    confirm_merge: bool,
    confirm_close: bool,
}

impl Default for ReviewView {
    fn default() -> Self {
        Self::new()
    }
}

impl ReviewView {
    pub fn new() -> Self {
        Self {
            mode: Mode::List,
            reviews: Vec::new(),
            cursor: 0,
            detail_scroll: 0,
            diff_lines: Vec::new(),
            diff_offset: 0,
            selected_review: None,
            comment_dialog: Dialog::new(""),
            confirm_merge: false,
            confirm_close: false,
        }
    }

    fn handle_list_event(&mut self, event: &KeyEvent, _ctx: &mut AppContext) -> Action {
        match event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.reviews.is_empty() && self.cursor < self.reviews.len() - 1 {
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
                if let Some(review) = self.reviews.get(self.cursor) {
                    self.selected_review = Some(review.clone());
                    self.detail_scroll = 0;
                    self.mode = Mode::Detail;
                }
                Action::Consumed
            }
            KeyCode::Char('r') => Action::Refresh,
            _ => Action::None,
        }
    }

    fn handle_detail_event(&mut self, event: &KeyEvent, ctx: &mut AppContext) -> Action {
        // Merge confirmation
        if self.confirm_merge {
            match event.code {
                KeyCode::Char('y') => {
                    self.confirm_merge = false;
                    if let Some(ref review) = self.selected_review {
                        match review::merge_review(&mut ctx.repo, review.id) {
                            Ok(updated) => {
                                self.selected_review = Some(updated);
                                Action::Success("Review merged!".into())
                            }
                            Err(e) => Action::Error(format!("Merge failed: {}", e)),
                        }
                    } else {
                        Action::Consumed
                    }
                }
                _ => {
                    self.confirm_merge = false;
                    Action::Consumed
                }
            }
        } else if self.confirm_close {
            match event.code {
                KeyCode::Char('y') => {
                    self.confirm_close = false;
                    if let Some(ref review) = self.selected_review {
                        match review::close_review(&ctx.repo, review.id) {
                            Ok(updated) => {
                                self.selected_review = Some(updated);
                                Action::Success("Review closed".into())
                            }
                            Err(e) => Action::Error(format!("Close failed: {}", e)),
                        }
                    } else {
                        Action::Consumed
                    }
                }
                _ => {
                    self.confirm_close = false;
                    Action::Consumed
                }
            }
        } else if self.comment_dialog.visible {
            match event.code {
                KeyCode::Enter => {
                    let body = self.comment_dialog.value().to_string();
                    self.comment_dialog.hide();
                    if body.trim().is_empty() {
                        return Action::Consumed;
                    }
                    if let Some(ref review) = self.selected_review {
                        match review::add_comment(
                            &ctx.repo,
                            review.id,
                            "(general)",
                            None,
                            &body,
                            None,
                        ) {
                            Ok(updated) => {
                                self.selected_review = Some(updated);
                                Action::Success("Comment added".into())
                            }
                            Err(e) => Action::Error(format!("Comment failed: {}", e)),
                        }
                    } else {
                        Action::Consumed
                    }
                }
                KeyCode::Esc => {
                    self.comment_dialog.hide();
                    Action::Consumed
                }
                _ => {
                    self.comment_dialog.input.handle_key(event);
                    Action::Consumed
                }
            }
        } else {
            match event.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.detail_scroll += 1;
                    Action::Consumed
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.detail_scroll = self.detail_scroll.saturating_sub(1);
                    Action::Consumed
                }
                KeyCode::Char('d') => {
                    // Load diff
                    if let Some(ref review) = self.selected_review {
                        match review::review_diff(&ctx.repo, review.id) {
                            Ok(changes) => {
                                self.diff_lines = changes
                                    .iter()
                                    .map(|c| {
                                        let marker = match c.kind {
                                            crate::fsmerkle::ChangeKind::Added => "++",
                                            crate::fsmerkle::ChangeKind::Deleted => "--",
                                            crate::fsmerkle::ChangeKind::Modified
                                            | crate::fsmerkle::ChangeKind::TypeChange => "~~",
                                        };
                                        format!("{} {}", marker, c.path)
                                    })
                                    .collect();
                                if self.diff_lines.is_empty() {
                                    self.diff_lines.push("No changes".into());
                                }
                                self.diff_offset = 0;
                                self.mode = Mode::Diff;
                            }
                            Err(e) => return Action::Error(format!("Diff failed: {}", e)),
                        }
                    }
                    Action::Consumed
                }
                KeyCode::Char('C') => {
                    self.comment_dialog.show("Add Comment");
                    Action::Consumed
                }
                KeyCode::Char('a') => {
                    if let Some(ref review) = self.selected_review {
                        match review::submit_verdict(
                            &ctx.repo,
                            review.id,
                            ReviewStatus::Approved,
                            "",
                        ) {
                            Ok(updated) => {
                                self.selected_review = Some(updated);
                                Action::Success("Review approved".into())
                            }
                            Err(e) => Action::Error(format!("Approve failed: {}", e)),
                        }
                    } else {
                        Action::Consumed
                    }
                }
                KeyCode::Char('x') => {
                    if let Some(ref review) = self.selected_review {
                        match review::submit_verdict(
                            &ctx.repo,
                            review.id,
                            ReviewStatus::ChangesRequested,
                            "",
                        ) {
                            Ok(updated) => {
                                self.selected_review = Some(updated);
                                Action::Success("Changes requested".into())
                            }
                            Err(e) => Action::Error(format!("Failed: {}", e)),
                        }
                    } else {
                        Action::Consumed
                    }
                }
                KeyCode::Char('m') => {
                    if let Some(ref review) = self.selected_review {
                        if review.status == ReviewStatus::Approved {
                            self.confirm_merge = true;
                        } else {
                            return Action::Error(format!(
                                "Review must be approved to merge (status: {})",
                                review.status
                            ));
                        }
                    }
                    Action::Consumed
                }
                KeyCode::Char('q') => {
                    // Close the review
                    if let Some(ref review) = self.selected_review
                        && review.status != ReviewStatus::Merged
                    {
                        self.confirm_close = true;
                    }
                    Action::Consumed
                }
                KeyCode::Esc => {
                    self.mode = Mode::List;
                    self.selected_review = None;
                    Action::Refresh
                }
                _ => Action::None,
            }
        }
    }

    fn handle_diff_event(&mut self, event: &KeyEvent) -> Action {
        match event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.diff_offset < self.diff_lines.len().saturating_sub(1) {
                    self.diff_offset += 1;
                }
                Action::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.diff_offset = self.diff_offset.saturating_sub(1);
                Action::Consumed
            }
            KeyCode::Char('g') => {
                self.diff_offset = 0;
                Action::Consumed
            }
            KeyCode::Char('G') => {
                self.diff_offset = self.diff_lines.len().saturating_sub(1);
                Action::Consumed
            }
            KeyCode::Esc => {
                self.mode = Mode::Detail;
                Action::Consumed
            }
            _ => Action::None,
        }
    }

    fn render_list(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.reviews.is_empty() {
            let msg = Paragraph::new(Span::styled(
                "No reviews. Create one with `ivaldi review create`",
                theme.dim,
            ));
            frame.render_widget(msg, area);
            return;
        }

        let inner_height = area.height.saturating_sub(3) as usize;

        let items: Vec<ListItem> = self
            .reviews
            .iter()
            .enumerate()
            .skip(self.cursor.saturating_sub(inner_height.saturating_sub(1)))
            .take(inner_height)
            .map(|(i, r)| {
                let marker = if i == self.cursor { ">" } else { " " };
                let text = format!(
                    "{} [{}] #{} {} ({} -> {})",
                    marker,
                    r.status.symbol(),
                    r.id,
                    r.title,
                    r.source_timeline,
                    r.target_timeline,
                );

                let style = if i == self.cursor {
                    theme.cursor
                } else {
                    match r.status {
                        ReviewStatus::Open => Style::default().fg(Color::White),
                        ReviewStatus::Approved => Style::default().fg(Color::Green),
                        ReviewStatus::ChangesRequested => Style::default().fg(Color::Yellow),
                        ReviewStatus::Merged => Style::default().fg(Color::Cyan),
                        ReviewStatus::Closed => theme.dim,
                    }
                };

                ListItem::new(Span::styled(text, style))
            })
            .collect();

        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            format!(" Reviews ({}) ", self.reviews.len()),
            theme.title,
        ));

        let list = List::new(items).block(block);
        frame.render_widget(list, area);

        // Help
        if area.height > 2 {
            let help_area = Rect {
                x: area.x,
                y: area.y + area.height - 1,
                width: area.width,
                height: 1,
            };
            let help = Paragraph::new(Span::styled(
                " j/k:navigate Enter:detail r:refresh",
                theme.dim,
            ));
            frame.render_widget(help, help_area);
        }
    }

    fn render_detail(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let review = match self.selected_review {
            Some(ref r) => r,
            None => return,
        };

        let mut lines = Vec::new();
        lines.push(Line::from(vec![
            Span::styled("Review #", theme.dim),
            Span::styled(format!("{}", review.id), theme.brand),
            Span::styled(": ", theme.dim),
            Span::styled(&review.title, Style::default().fg(Color::White).bold()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Status:  ", theme.dim),
            Span::styled(
                format!("{}", review.status),
                match review.status {
                    ReviewStatus::Open => Style::default().fg(Color::White),
                    ReviewStatus::Approved => Style::default().fg(Color::Green),
                    ReviewStatus::ChangesRequested => Style::default().fg(Color::Yellow),
                    ReviewStatus::Merged => Style::default().fg(Color::Cyan),
                    ReviewStatus::Closed => theme.dim,
                },
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Author:  ", theme.dim),
            Span::styled(&review.author, Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Source:  ", theme.dim),
            Span::styled(&review.source_timeline, theme.brand),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Target:  ", theme.dim),
            Span::styled(&review.target_timeline, theme.brand),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Strategy: ", theme.dim),
            Span::styled(&review.fuse_strategy, Style::default().fg(Color::White)),
        ]));

        if let Some(ref seal) = review.merge_seal {
            lines.push(Line::from(vec![
                Span::styled("Merged:  ", theme.dim),
                Span::styled(seal.as_str(), Style::default().fg(Color::Green)),
            ]));
        }

        if !review.description.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                &review.description,
                Style::default().fg(Color::White),
            )));
        }

        // Comments
        if !review.comments.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("--- Comments ({}) ---", review.comments.len()),
                theme.dim,
            )));
            for c in &review.comments {
                let location = if let Some(line) = c.line {
                    format!("{}:{}", c.path, line)
                } else {
                    c.path.clone()
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  [{}] ", c.id), theme.dim),
                    Span::styled(&c.author, Style::default().fg(Color::Yellow)),
                    Span::styled(" @ ", theme.dim),
                    Span::styled(location, theme.brand),
                ]));
                lines.push(Line::from(Span::styled(
                    format!("    {}", c.body),
                    Style::default().fg(Color::White),
                )));
            }
        }

        // Verdicts
        if !review.verdicts.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("--- Verdicts ({}) ---", review.verdicts.len()),
                theme.dim,
            )));
            for v in &review.verdicts {
                let status_style = match v.status {
                    ReviewStatus::Approved => Style::default().fg(Color::Green),
                    ReviewStatus::ChangesRequested => Style::default().fg(Color::Yellow),
                    _ => Style::default().fg(Color::White),
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {} ", v.status), status_style),
                    Span::styled("- ", theme.dim),
                    Span::styled(&v.author, Style::default().fg(Color::White)),
                    if v.body.is_empty() {
                        Span::raw("")
                    } else {
                        Span::styled(format!(" {}", v.body), theme.dim)
                    },
                ]));
            }
        }

        let visible_lines: Vec<Line> = lines.into_iter().skip(self.detail_scroll).collect();

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Review Detail ", theme.title));

        let paragraph = Paragraph::new(visible_lines).block(block);
        frame.render_widget(paragraph, area);

        // Confirmation overlays
        if self.confirm_merge {
            let msg_area = Rect {
                x: area.x + 2,
                y: area.y + area.height.saturating_sub(2),
                width: area.width.saturating_sub(4),
                height: 1,
            };
            let msg = Paragraph::new(Span::styled(
                "Merge this review? y:yes any:cancel",
                theme.warning,
            ));
            frame.render_widget(msg, msg_area);
        } else if self.confirm_close {
            let msg_area = Rect {
                x: area.x + 2,
                y: area.y + area.height.saturating_sub(2),
                width: area.width.saturating_sub(4),
                height: 1,
            };
            let msg = Paragraph::new(Span::styled(
                "Close this review? y:yes any:cancel",
                theme.warning,
            ));
            frame.render_widget(msg, msg_area);
        } else {
            // Help
            if area.height > 2 {
                let help_area = Rect {
                    x: area.x,
                    y: area.y + area.height - 1,
                    width: area.width,
                    height: 1,
                };
                let help = Paragraph::new(Span::styled(
                    " j/k:scroll d:diff C:comment a:approve x:request-changes m:merge q:close Esc:back",
                    theme.dim,
                ));
                frame.render_widget(help, help_area);
            }
        }

        // Comment dialog overlay
        self.comment_dialog.render(frame, frame.area(), theme);
    }

    fn render_diff(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let inner_height = area.height.saturating_sub(3) as usize;

        let lines: Vec<Line> = self
            .diff_lines
            .iter()
            .skip(self.diff_offset)
            .take(inner_height)
            .map(|line| {
                let style = if line.starts_with("++") {
                    Style::default().fg(Color::Green)
                } else if line.starts_with("--") {
                    Style::default().fg(Color::Red)
                } else if line.starts_with("~~") {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::White)
                };
                Line::from(Span::styled(line.as_str(), style))
            })
            .collect();

        let title = if let Some(ref r) = self.selected_review {
            format!(" Diff — Review #{} ", r.id)
        } else {
            " Diff ".into()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(title, theme.title));

        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(paragraph, area);

        // Help
        if area.height > 2 {
            let help_area = Rect {
                x: area.x,
                y: area.y + area.height - 1,
                width: area.width,
                height: 1,
            };
            let help = Paragraph::new(Span::styled(
                " j/k:scroll g/G:top/bottom Esc:back",
                theme.dim,
            ));
            frame.render_widget(help, help_area);
        }
    }
}

impl TabView for ReviewView {
    fn handle_event(&mut self, event: &KeyEvent, ctx: &mut AppContext) -> Action {
        match self.mode {
            Mode::List => self.handle_list_event(event, ctx),
            Mode::Detail => self.handle_detail_event(event, ctx),
            Mode::Diff => self.handle_diff_event(event),
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        match self.mode {
            Mode::List => self.render_list(frame, area, theme),
            Mode::Detail => self.render_detail(frame, area, theme),
            Mode::Diff => self.render_diff(frame, area, theme),
        }
    }

    fn load_data(&mut self, ctx: &AppContext) {
        self.reviews =
            review::list_reviews(&ctx.repo, &ReviewFilter::default()).unwrap_or_default();
        if self.cursor >= self.reviews.len() && !self.reviews.is_empty() {
            self.cursor = self.reviews.len() - 1;
        }
        // Refresh selected review if in detail/diff mode
        if let Some(ref current) = self.selected_review
            && let Ok(Some(updated)) = ctx.repo.load_review(current.id)
        {
            self.selected_review = Some(updated);
        }
    }

    fn short_help(&self) -> &str {
        match self.mode {
            Mode::List => "Enter:detail r:refresh",
            Mode::Detail => "d:diff C:comment a:approve x:changes m:merge",
            Mode::Diff => "j/k:scroll g/G:top/bottom Esc:back",
        }
    }

    fn has_active_input(&self) -> bool {
        self.comment_dialog.visible || self.confirm_merge || self.confirm_close
    }
}
