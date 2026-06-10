//! Status tab — staging, sealing, file state display.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::ignore;
use crate::tui::components::dialog::Dialog;
use crate::tui::components::file_list::{FileItem, FileListWidget};
use crate::tui::theme::Theme;
use crate::tui::types::{Action, AppContext};
use crate::tui::views::TabView;
use crate::workspace::{DotfileAllowlist, FileState, StagingArea, Workspace};

/// What action the dialog will perform when the user submits with Enter.
///
/// The `Dialog` widget is generic, so the StatusView remembers which prompt
/// it last opened in order to dispatch the right business-logic call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogPurpose {
    Seal,
    ConfirmReverseAll,
    /// Add a glob pattern to .ivaldiignore. Same logic as `ivaldi exclude`.
    AddIgnorePattern,
}

pub struct StatusView {
    file_list: FileListWidget,
    dialog: Dialog,
    dialog_purpose: DialogPurpose,
    show_ignored: bool,
    message: Option<String>,
}

impl Default for StatusView {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusView {
    pub fn new() -> Self {
        Self {
            file_list: FileListWidget::new(),
            dialog: Dialog::new("Seal Message"),
            dialog_purpose: DialogPurpose::Seal,
            show_ignored: false,
            message: None,
        }
    }

    fn gather_selected(&mut self, ctx: &mut AppContext) -> Action {
        let paths: Vec<String> = self
            .file_list
            .items
            .iter()
            .filter(|i| i.selected && !matches!(i.state, FileState::Staged))
            .map(|i| i.path.clone())
            .collect();

        if paths.is_empty() {
            // If nothing selected, gather current item
            if let Some(item) = self.file_list.current_item()
                && !matches!(item.state, FileState::Staged)
            {
                let path = item.path.clone();
                return self.do_gather(ctx, &[path]);
            }
            return Action::Consumed;
        }

        self.do_gather(ctx, &paths)
    }

    fn do_gather(&mut self, ctx: &mut AppContext, paths: &[String]) -> Action {
        let mut ws = Workspace::new(&ctx.repo.cas, &ctx.work_dir, &ctx.ivaldi_dir);
        ws.staging = StagingArea::load(&ctx.ivaldi_dir);
        let allowlist = DotfileAllowlist::load(&ctx.ivaldi_dir);

        let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        match ws.gather(&path_refs, &allowlist) {
            Ok(result) => {
                if let Err(e) = ws.staging.save(&ctx.ivaldi_dir) {
                    return Action::Error(format!("Failed to save staging: {}", e));
                }
                self.message = Some(format!("Gathered {} file(s)", result.gathered.len()));
                Action::Refresh
            }
            Err(e) => Action::Error(format!("Gather failed: {}", e)),
        }
    }

    fn ungather_selected(&mut self, ctx: &mut AppContext) -> Action {
        let mut staging = StagingArea::load(&ctx.ivaldi_dir);
        let mut count = 0usize;

        let paths: Vec<String> = self
            .file_list
            .items
            .iter()
            .filter(|i| i.selected && matches!(i.state, FileState::Staged))
            .map(|i| i.path.clone())
            .collect();

        if paths.is_empty() {
            // Ungather current
            if let Some(item) = self.file_list.current_item()
                && matches!(item.state, FileState::Staged)
            {
                staging.unstage(&item.path);
                count = 1;
            }
        } else {
            for path in &paths {
                if staging.unstage(path) {
                    count += 1;
                }
            }
        }

        if count > 0 {
            if let Err(e) = staging.save(&ctx.ivaldi_dir) {
                return Action::Error(format!("Failed to save staging: {}", e));
            }
            self.message = Some(format!("Ungathered {} file(s)", count));
            Action::Refresh
        } else {
            Action::Consumed
        }
    }

    fn gather_all(&mut self, ctx: &mut AppContext) -> Action {
        let mut ws = Workspace::new(&ctx.repo.cas, &ctx.work_dir, &ctx.ivaldi_dir);
        ws.staging = StagingArea::load(&ctx.ivaldi_dir);
        let ignore = ignore::load_pattern_cache(&ctx.work_dir);

        match ws.gather_all(&ignore) {
            Ok(result) => {
                if let Err(e) = ws.staging.save(&ctx.ivaldi_dir) {
                    return Action::Error(format!("Failed to save staging: {}", e));
                }
                self.message = Some(format!("Gathered {} file(s)", result.gathered.len()));
                Action::Refresh
            }
            Err(e) => Action::Error(format!("Gather all failed: {}", e)),
        }
    }

    fn do_seal(&mut self, ctx: &mut AppContext) -> Action {
        let message = self.dialog.value().to_string();
        self.dialog.hide();

        if message.trim().is_empty() {
            return Action::Error("Seal message cannot be empty".into());
        }

        let staging = StagingArea::load(&ctx.ivaldi_dir);
        if staging.is_empty() {
            return Action::Error("Nothing to seal (staging area is empty)".into());
        }

        // Resolve the current timeline's tip tree so the new seal inherits
        // unchanged files from the parent rather than dropping them.
        let timeline = match ctx.repo.current_timeline() {
            Ok(t) => t,
            Err(e) => return Action::Error(format!("Failed to read HEAD: {}", e)),
        };
        let parent_tree = match ctx.repo.get_timeline_head(&timeline) {
            Ok(Some(idx)) => match ctx.repo.get_leaf(idx) {
                Ok(Some(leaf)) => Some(leaf.tree_root),
                _ => None,
            },
            _ => None,
        };

        let ws = Workspace::new(&ctx.repo.cas, &ctx.work_dir, &ctx.ivaldi_dir);
        let tree_root = match ws.build_seal_tree(parent_tree) {
            Ok(h) => h,
            Err(e) => return Action::Error(format!("Failed to build tree: {}", e)),
        };

        let config = crate::config::load_config(&ctx.ivaldi_dir);
        let author = config
            .author()
            .unwrap_or_else(|| "unknown <unknown>".into());

        match ctx.repo.commit(tree_root, &author, &message) {
            Ok(result) => {
                // Clear staging after successful seal
                let empty = StagingArea::new();
                let _ = empty.save(&ctx.ivaldi_dir);
                self.message = Some(format!("Sealed: {}", result.seal_name));
                Action::Refresh
            }
            Err(e) => Action::Error(format!("Seal failed: {}", e)),
        }
    }

    fn do_add_ignore_pattern(&mut self, ctx: &mut AppContext) -> Action {
        let pattern = self.dialog.value().trim().to_string();
        self.dialog.hide();

        if pattern.is_empty() {
            return Action::Consumed;
        }

        // Same append-if-absent logic as `ivaldi exclude`.
        let path = ctx.work_dir.join(".ivaldiignore");
        let mut content = std::fs::read_to_string(&path).unwrap_or_default();
        if content.lines().any(|l| l.trim() == pattern) {
            self.message = Some(format!("Already in .ivaldiignore: {}", pattern));
            return Action::Refresh;
        }
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&pattern);
        content.push('\n');
        if let Err(e) = std::fs::write(&path, &content) {
            return Action::Error(format!("Failed to write .ivaldiignore: {}", e));
        }
        self.message = Some(format!("Added to .ivaldiignore: {}", pattern));
        Action::Refresh
    }

    fn do_reverse_all(&mut self, ctx: &mut AppContext) -> Action {
        let confirmation = self.dialog.value().trim().to_string();
        self.dialog.hide();
        if !confirmation.eq_ignore_ascii_case("yes") {
            self.message = Some("Reverse cancelled".into());
            return Action::Consumed;
        }

        // Resolve the current timeline tip's tree and materialize it.
        let timeline = match ctx.repo.current_timeline() {
            Ok(t) => t,
            Err(e) => return Action::Error(format!("Failed to read HEAD: {}", e)),
        };
        let tree_root = match ctx.repo.get_timeline_head(&timeline) {
            Ok(Some(idx)) => match ctx.repo.get_leaf(idx) {
                Ok(Some(leaf)) => leaf.tree_root,
                Ok(None) => return Action::Error("Timeline tip not found".into()),
                Err(e) => return Action::Error(format!("Failed to load tip: {}", e)),
            },
            Ok(None) => return Action::Error("Timeline has no commits yet".into()),
            Err(e) => return Action::Error(format!("Failed to read timeline: {}", e)),
        };

        let ws = Workspace::new(&ctx.repo.cas, &ctx.work_dir, &ctx.ivaldi_dir);
        if let Err(e) = ws.materialize(tree_root) {
            return Action::Error(format!("Reverse failed: {}", e));
        }

        // Clear staging too — `reverse --all` throws away everything.
        let empty = StagingArea::new();
        let _ = empty.save(&ctx.ivaldi_dir);

        self.message = Some("Reversed all changes; restored to tip".into());
        Action::Refresh
    }
}

impl TabView for StatusView {
    fn handle_event(&mut self, event: &KeyEvent, ctx: &mut AppContext) -> Action {
        // Dialog mode
        if self.dialog.visible {
            match event.code {
                KeyCode::Enter => match self.dialog_purpose {
                    DialogPurpose::Seal => return self.do_seal(ctx),
                    DialogPurpose::ConfirmReverseAll => return self.do_reverse_all(ctx),
                    DialogPurpose::AddIgnorePattern => return self.do_add_ignore_pattern(ctx),
                },
                KeyCode::Esc => {
                    self.dialog.hide();
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
                self.file_list.move_down();
                Action::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.file_list.move_up();
                Action::Consumed
            }
            KeyCode::Char('g') => {
                self.file_list.move_top();
                Action::Consumed
            }
            KeyCode::Char('G') => {
                self.file_list.move_bottom();
                Action::Consumed
            }
            KeyCode::Char(' ') => self.gather_selected(ctx),
            KeyCode::Char('u') => self.ungather_selected(ctx),
            KeyCode::Char('a') => self.gather_all(ctx),
            KeyCode::Char('s') => {
                self.dialog_purpose = DialogPurpose::Seal;
                self.dialog.show("Seal Message");
                Action::Consumed
            }
            KeyCode::Char('R') => {
                self.dialog_purpose = DialogPurpose::ConfirmReverseAll;
                self.dialog
                    .show("Reverse all? Type 'yes' to throw away all working-tree changes");
                Action::Consumed
            }
            KeyCode::Char('E') => {
                // Pre-fill the dialog with the path under the cursor so the
                // common case (ignore this untracked file) is one keystroke.
                let preset = self
                    .file_list
                    .current_item()
                    .map(|i| i.path.clone())
                    .unwrap_or_default();
                self.dialog_purpose = DialogPurpose::AddIgnorePattern;
                if preset.is_empty() {
                    self.dialog.show("Add .ivaldiignore pattern");
                } else {
                    self.dialog
                        .show_with_value("Add .ivaldiignore pattern", preset);
                }
                Action::Consumed
            }
            KeyCode::Char('i') => {
                self.show_ignored = !self.show_ignored;
                Action::Refresh
            }
            KeyCode::Char('r') => Action::Refresh,
            _ => Action::None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.file_list.items.is_empty() {
            let msg = Paragraph::new(Span::styled("Working directory clean", theme.success));
            frame.render_widget(msg, area);
        } else {
            self.file_list.render(frame, area, "Files", theme);
        }

        // Help line at bottom of file list
        if area.height > 2 {
            let help_area = Rect {
                x: area.x,
                y: area.y + area.height - 1,
                width: area.width,
                height: 1,
            };
            let help = Paragraph::new(Span::styled(
                " Space:gather  u:ungather  a:all  s:seal  R:reverse-all  E:exclude  i:ignored  r:refresh",
                theme.dim,
            ));
            frame.render_widget(help, help_area);
        }

        // Dialog overlay
        self.dialog.render(frame, frame.area(), theme);
    }

    fn load_data(&mut self, ctx: &AppContext) {
        let ignore = ignore::load_pattern_cache(&ctx.work_dir);
        let staging = StagingArea::load(&ctx.ivaldi_dir);
        let ws = Workspace::new(&ctx.repo.cas, &ctx.work_dir, &ctx.ivaldi_dir);

        let timeline = ctx.repo.current_timeline().unwrap_or_default();
        let last_tree = ctx
            .repo
            .walk_history(&timeline)
            .ok()
            .and_then(|h| {
                h.first().map(|e| {
                    // Get the tree root from the leaf
                    ctx.repo
                        .get_leaf(e.index)
                        .ok()
                        .flatten()
                        .map(|l| l.tree_root)
                })
            })
            .flatten();

        let files = ws.status(last_tree, &ignore).unwrap_or_default();

        let items: Vec<FileItem> = files
            .into_iter()
            .filter(|f| {
                if !self.show_ignored {
                    !matches!(f.state, FileState::Unmodified)
                } else {
                    true
                }
            })
            .map(|f| {
                let state = if staging.is_staged(&f.path) {
                    FileState::Staged
                } else {
                    f.state
                };
                FileItem {
                    path: f.path,
                    state,
                    selected: false,
                }
            })
            .collect();

        self.file_list.set_items(items);
    }

    fn short_help(&self) -> &str {
        "Space:gather u:ungather a:all s:seal R:reverse-all E:exclude i:ignored"
    }

    fn has_active_input(&self) -> bool {
        self.dialog.visible
    }
}
