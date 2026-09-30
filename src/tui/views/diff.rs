//! Diff tab — working/staged changes viewer.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

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

impl Default for DiffTabView {
    fn default() -> Self {
        Self::new()
    }
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
            .get_timeline_head(&timeline)
            .ok()
            .flatten()
            .and_then(|idx| ctx.repo.get_leaf(idx).ok().flatten())
            .map(|leaf| leaf.tree_root);

        // The "before" side of every diff is the last seal's version of the
        // file, looked up by path — `status` reports the working file's own
        // hash, which names the new content, not the old.
        let sealed = last_tree
            .and_then(|tree| ws.list_tree_files(tree).ok())
            .unwrap_or_default();
        let store = crate::fsmerkle::FsStore::new(&ctx.repo.cas);
        let read_blob = |hash: &crate::hash::B3Hash| {
            store
                .load_blob(*hash)
                .ok()
                .and_then(|(_, data)| String::from_utf8(data).ok())
        };

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
                    if let Some(content) = sealed.get(file_path).and_then(read_blob) {
                        for line in content.lines() {
                            diff_lines.push(DiffLine {
                                kind: DiffLineKind::Remove,
                                text: format!("-{}", line),
                            });
                        }
                    }
                }
                FileState::Modified | FileState::Staged => {
                    // Staged view shows what the next seal records: the staged
                    // blob, which may differ from the file edited since.
                    let staged = matches!(file.state, FileState::Staged)
                        .then(|| ws.staging.staged_files().get(file_path))
                        .flatten();
                    let new_content = match staged {
                        Some(hash) => read_blob(hash).unwrap_or_default(),
                        None => std::fs::read_to_string(&full_path).unwrap_or_default(),
                    };
                    let old_content = sealed
                        .get(file_path)
                        .and_then(read_blob)
                        .unwrap_or_default();

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

#[cfg(test)]
mod tests {
    use super::*;

    fn seal(repo: &mut crate::repo::Repo, files: &[(&str, &str)]) {
        let files: std::collections::BTreeMap<String, Vec<u8>> = files
            .iter()
            .map(|(path, content)| (path.to_string(), content.as_bytes().to_vec()))
            .collect();
        let tree = crate::fsmerkle::FsStore::new(&repo.cas)
            .build_tree_from_map(&files)
            .unwrap();
        repo.commit(tree, "author", "seal").unwrap();
    }

    fn setup(dir: &std::path::Path) -> AppContext {
        crate::forge::forge(dir).unwrap();
        let mut repo = crate::repo::Repo::open(dir).unwrap();
        seal(
            &mut repo,
            &[("a.txt", "one\ntwo\nthree\n"), ("gone.txt", "bye\n")],
        );
        std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        AppContext {
            work_dir: dir.to_path_buf(),
            ivaldi_dir: dir.join(".ivaldi"),
            repo,
        }
    }

    fn rendered(view: &DiffTabView) -> Vec<(DiffLineKind, String)> {
        view.diff_view
            .lines
            .iter()
            .map(|l| (l.kind, l.text.clone()))
            .collect()
    }

    fn changed(lines: &[(DiffLineKind, String)]) -> Vec<String> {
        lines
            .iter()
            .filter(|(kind, _)| matches!(kind, DiffLineKind::Add | DiffLineKind::Remove))
            .map(|(_, text)| text.clone())
            .collect()
    }

    #[test]
    fn modified_file_diffs_against_the_sealed_version() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = setup(dir.path());
        let mut view = DiffTabView::new();
        view.load_data(&ctx);
        let lines = rendered(&view);
        // Only the edited line, not the whole file as added.
        assert_eq!(changed(&lines), ["-two", "+TWO", "-bye"]);
    }

    #[test]
    fn staged_view_shows_the_staged_blob_against_the_sealed_version() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = setup(dir.path());
        let mut ws = Workspace::new(&ctx.repo.cas, dir.path(), dir.path().join(".ivaldi"));
        ws.gather(
            &["a.txt"],
            &crate::workspace::DotfileAllowlist::load(&dir.path().join(".ivaldi")),
        )
        .unwrap();
        ws.save().unwrap();
        // Edited again after staging: the staged view must not show this.
        std::fs::write(dir.path().join("a.txt"), "one\nTWO\nthree\nfour\n").unwrap();

        let mut view = DiffTabView::new();
        view.show_staged = true;
        view.load_data(&ctx);
        assert_eq!(changed(&rendered(&view)), ["-two", "+TWO"]);
    }
}
