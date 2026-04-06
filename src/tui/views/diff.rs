//! Diff tab — working/staged changes viewer.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::cas::Cas;
use crate::ignore;
use crate::tui::components::diff_view::{
    DiffLine, DiffLineKind, DiffViewWidget, compute_line_diff,
};
use crate::tui::theme::Theme;
use crate::tui::types::{Action, AppContext};
use crate::tui::views::TabView;
use crate::workspace::{FileState, Workspace};

pub struct DiffTabView {
    diff_view: DiffViewWidget,
    show_staged: bool,
}

impl DiffTabView {
    pub fn new() -> Self {
        Self {
            diff_view: DiffViewWidget::new(),
            show_staged: false,
        }
    }
}

impl TabView for DiffTabView {
    fn handle_event(&mut self, event: &KeyEvent, _ctx: &mut AppContext) -> Action {
        match event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.diff_view.scroll_down(1);
                Action::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.diff_view.scroll_up(1);
                Action::Consumed
            }
            KeyCode::Char('n') => {
                self.diff_view.next_file();
                Action::Consumed
            }
            KeyCode::Char('p') => {
                self.diff_view.prev_file();
                Action::Consumed
            }
            KeyCode::Char('g') => {
                self.diff_view.scroll_top();
                Action::Consumed
            }
            KeyCode::Char('G') => {
                self.diff_view.scroll_bottom();
                Action::Consumed
            }
            KeyCode::Char('s') => {
                self.show_staged = !self.show_staged;
                Action::Refresh
            }
            KeyCode::PageDown => {
                self.diff_view.page_down(20);
                Action::Consumed
            }
            KeyCode::PageUp => {
                self.diff_view.page_up(20);
                Action::Consumed
            }
            KeyCode::Char('r') => Action::Refresh,
            _ => Action::None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.diff_view.lines.is_empty() {
            let mode = if self.show_staged {
                "staged"
            } else {
                "working"
            };
            let msg = Paragraph::new(Span::styled(format!("No {} changes", mode), theme.dim));
            frame.render_widget(msg, area);
        } else {
            self.diff_view.render(frame, area, theme);
        }

        // Help at bottom
        if area.height > 2 {
            let help_area = Rect {
                x: area.x,
                y: area.y + area.height - 1,
                width: area.width,
                height: 1,
            };
            let mode = if self.show_staged {
                "staged"
            } else {
                "working"
            };
            let help = Paragraph::new(Span::styled(
                format!(
                    " j/k:scroll n/p:file s:toggle({}) g/G:top/bottom r:refresh",
                    mode
                ),
                theme.dim,
            ));
            frame.render_widget(help, help_area);
        }
    }

    fn load_data(&mut self, ctx: &AppContext) {
        let ignore = ignore::load_pattern_cache(&ctx.work_dir);
        let ws = Workspace::new(&ctx.repo.cas, &ctx.work_dir, &ctx.ivaldi_dir);

        let timeline = ctx.repo.current_timeline().unwrap_or_default();
        let last_tree = ctx
            .repo
            .walk_history(&timeline)
            .ok()
            .and_then(|h| {
                h.first().map(|e| {
                    ctx.repo
                        .get_leaf(e.index)
                        .ok()
                        .flatten()
                        .map(|l| l.tree_root)
                })
            })
            .flatten();

        let files = ws.status(last_tree, &ignore).unwrap_or_default();

        let mut diff_lines: Vec<DiffLine> = Vec::new();

        for file in &files {
            let dominated = if self.show_staged {
                matches!(file.state, FileState::Staged)
            } else {
                matches!(
                    file.state,
                    FileState::Modified | FileState::Untracked | FileState::Deleted
                )
            };

            if !dominated {
                continue;
            }

            let file_path = &file.path;
            let full_path = ctx.work_dir.join(file_path);

            match file.state {
                FileState::Untracked => {
                    diff_lines.push(DiffLine {
                        kind: DiffLineKind::Header,
                        text: format!("=== new file: {}", file_path),
                    });
                    if let Ok(content) = std::fs::read_to_string(&full_path) {
                        for line in content.lines() {
                            diff_lines.push(DiffLine {
                                kind: DiffLineKind::Add,
                                text: format!("+{}", line),
                            });
                        }
                    }
                }
                FileState::Deleted => {
                    diff_lines.push(DiffLine {
                        kind: DiffLineKind::Header,
                        text: format!("=== deleted: {}", file_path),
                    });
                    // Try to read from CAS using last known hash
                    if let Some(hash) = file.hash {
                        if let Ok(data) = ctx.repo.cas.get(hash) {
                            if let Ok(content) = String::from_utf8(data) {
                                for line in content.lines() {
                                    diff_lines.push(DiffLine {
                                        kind: DiffLineKind::Remove,
                                        text: format!("-{}", line),
                                    });
                                }
                            }
                        }
                    }
                }
                FileState::Modified | FileState::Staged => {
                    let new_content = std::fs::read_to_string(&full_path).unwrap_or_default();

                    // Try to get old content from CAS
                    let old_content = if let Some(hash) = file.hash {
                        ctx.repo
                            .cas
                            .get(hash)
                            .ok()
                            .and_then(|d| String::from_utf8(d).ok())
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };

                    let file_diff = compute_line_diff(&old_content, &new_content, file_path);
                    diff_lines.extend(file_diff);
                }
                FileState::Unmodified => {}
            }
        }

        self.diff_view.set_lines(diff_lines);
    }

    fn short_help(&self) -> &str {
        "j/k:scroll n/p:file s:staged/working g/G:top/bottom"
    }

    fn has_active_input(&self) -> bool {
        false
    }
}
