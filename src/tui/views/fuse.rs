//! Fuse (merge) tab — merge timelines together.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::fuse::Strategy;
use crate::tui::theme::Theme;
use crate::tui::types::{Action, AppContext};
use crate::tui::views::TabView;

const STRATEGIES: [Strategy; 5] = [
    Strategy::Auto,
    Strategy::Ours,
    Strategy::Theirs,
    Strategy::Union,
    Strategy::Base,
];

pub struct FuseView {
    timelines: Vec<(String, bool)>, // name, is_current — excludes current
    cursor: usize,
    strategy_idx: usize,
    merge_in_progress: bool,
    merge_conflicts: Vec<String>,
    confirm_abort: bool,
}

impl FuseView {
    pub fn new() -> Self {
        Self {
            timelines: Vec::new(),
            cursor: 0,
            strategy_idx: 0,
            merge_in_progress: false,
            merge_conflicts: Vec::new(),
            confirm_abort: false,
        }
    }

    fn current_strategy(&self) -> Strategy {
        STRATEGIES[self.strategy_idx]
    }

    fn do_fuse(&mut self, ctx: &mut AppContext) -> Action {
        let source_name = match self.timelines.get(self.cursor) {
            Some((name, _)) => name.clone(),
            None => return Action::Error("No timeline selected".into()),
        };

        let current = match ctx.repo.current_timeline() {
            Ok(t) => t,
            Err(e) => return Action::Error(format!("Failed: {}", e)),
        };

        // Get tree hashes for both timelines
        let our_tree = match self.get_head_tree(ctx, &current) {
            Some(h) => h,
            None => return Action::Error("Current timeline has no commits".into()),
        };
        let their_tree = match self.get_head_tree(ctx, &source_name) {
            Some(h) => h,
            None => return Action::Error(format!("Timeline '{}' has no commits", source_name)),
        };

        // Find common base (simplified: use empty tree as base)
        let base = std::collections::BTreeMap::new();

        // Load tree contents
        let ours = self.load_tree_map(ctx, our_tree);
        let theirs = self.load_tree_map(ctx, their_tree);

        let result = crate::fuse::FuseEngine::fuse(&base, &ours, &theirs, self.current_strategy());

        if result.success {
            // Apply merged files
            let config = crate::config::load_config(&ctx.ivaldi_dir);
            let author = config.author().unwrap_or_else(|| "unknown <unknown>".into());
            let msg = format!("Fuse {} into {}", source_name, current);

            // Build tree from merged file hashes and commit
            let store = crate::fsmerkle::FsStore::new(&ctx.repo.cas);

            match store.build_tree_from_hash_map(&result.merged_files) {
                Ok(tree_root) => match ctx.repo.commit(tree_root, &author, &msg) {
                    Ok(cr) => {
                        let _ = ctx.repo.clear_merge_state();
                        Action::Success(format!("Fuse complete: {}", cr.seal_name))
                    }
                    Err(e) => Action::Error(format!("Commit failed: {}", e)),
                },
                Err(e) => Action::Error(format!("Tree build failed: {}", e)),
            }
        } else {
            // Save merge state with conflicts
            let conflicts: Vec<String> = result.conflicts.iter().map(|c| c.path.clone()).collect();
            let state = crate::repo::MergeState {
                source_timeline: source_name,
                target_timeline: current,
                strategy: self.current_strategy().to_string(),
                conflicts: conflicts.clone(),
            };
            match ctx.repo.save_merge_state(&state) {
                Ok(()) => {
                    self.merge_in_progress = true;
                    self.merge_conflicts = conflicts;
                    Action::Error(format!(
                        "{} conflict(s) found. Resolve and continue, or abort.",
                        result.conflicts.len()
                    ))
                }
                Err(e) => Action::Error(format!("Failed to save merge state: {}", e)),
            }
        }
    }

    fn get_head_tree(&self, ctx: &AppContext, timeline: &str) -> Option<crate::hash::B3Hash> {
        ctx.repo
            .walk_history(timeline)
            .ok()?
            .first()
            .and_then(|e| ctx.repo.get_leaf(e.index).ok().flatten().map(|l| l.tree_root))
    }

    fn load_tree_map(
        &self,
        ctx: &AppContext,
        tree_hash: crate::hash::B3Hash,
    ) -> std::collections::BTreeMap<String, crate::hash::B3Hash> {
        let store = crate::fsmerkle::FsStore::new(&ctx.repo.cas);
        let mut map = std::collections::BTreeMap::new();
        let _ = Self::collect_tree(&store, tree_hash, "", &mut map);
        map
    }

    fn collect_tree(
        store: &crate::fsmerkle::FsStore<'_>,
        hash: crate::hash::B3Hash,
        prefix: &str,
        map: &mut std::collections::BTreeMap<String, crate::hash::B3Hash>,
    ) -> Result<(), String> {
        let tree = store.load_tree(hash).map_err(|e| e.to_string())?;
        for entry in &tree.entries {
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{}/{}", prefix, entry.name)
            };
            if entry.kind == crate::fsmerkle::NodeKind::Tree {
                let _ = Self::collect_tree(store, entry.hash, &path, map);
            } else {
                map.insert(path, entry.hash);
            }
        }
        Ok(())
    }
}

impl TabView for FuseView {
    fn handle_event(&mut self, event: &KeyEvent, ctx: &mut AppContext) -> Action {
        // Abort confirmation
        if self.confirm_abort {
            match event.code {
                KeyCode::Char('y') => {
                    self.confirm_abort = false;
                    match ctx.repo.clear_merge_state() {
                        Ok(()) => {
                            self.merge_in_progress = false;
                            self.merge_conflicts.clear();
                            Action::Success("Merge aborted".into())
                        }
                        Err(e) => Action::Error(format!("Abort failed: {}", e)),
                    }
                }
                _ => {
                    self.confirm_abort = false;
                    Action::Consumed
                }
            }
        } else {
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
                KeyCode::Enter | KeyCode::Char('f') => self.do_fuse(ctx),
                KeyCode::Char('s') => {
                    self.strategy_idx = (self.strategy_idx + 1) % STRATEGIES.len();
                    Action::Consumed
                }
                KeyCode::Char('a') => {
                    if self.merge_in_progress {
                        self.confirm_abort = true;
                    }
                    Action::Consumed
                }
                KeyCode::Char('r') => Action::Refresh,
                _ => Action::None,
            }
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        // Merge in progress banner
        if self.merge_in_progress {
            let banner_area = Rect {
                height: 2.min(area.height),
                ..area
            };
            let banner = Paragraph::new(vec![
                Line::from(Span::styled(
                    "MERGE IN PROGRESS",
                    theme.warning,
                )),
                Line::from(Span::styled(
                    format!("Conflicts: {}", self.merge_conflicts.join(", ")),
                    theme.error,
                )),
            ]);
            frame.render_widget(banner, banner_area);
        }

        // Strategy indicator
        let strategy_y = if self.merge_in_progress {
            area.y + 2
        } else {
            area.y
        };
        let strategy_area = Rect {
            x: area.x,
            y: strategy_y,
            width: area.width,
            height: 1,
        };
        let strategy_text = Paragraph::new(Line::from(vec![
            Span::styled("Strategy: ", theme.dim),
            Span::styled(
                format!("{}", self.current_strategy()),
                theme.brand,
            ),
            Span::styled("  (press 's' to cycle)", theme.dim),
        ]));
        frame.render_widget(strategy_text, strategy_area);

        // Timeline list
        let list_y = strategy_y + 1;
        let list_height = area
            .height
            .saturating_sub(list_y - area.y)
            .saturating_sub(1);
        let list_area = Rect {
            x: area.x,
            y: list_y,
            width: area.width,
            height: list_height,
        };

        if self.timelines.is_empty() {
            let msg = Paragraph::new(Span::styled(
                "No other timelines to fuse",
                theme.dim,
            ));
            frame.render_widget(msg, list_area);
        } else {
            let items: Vec<ListItem> = self
                .timelines
                .iter()
                .enumerate()
                .map(|(i, (name, _))| {
                    let marker = if i == self.cursor { ">" } else { " " };
                    let text = format!("{} {}", marker, name);
                    let style = if i == self.cursor {
                        theme.cursor
                    } else {
                        Style::default().fg(Color::White)
                    };
                    ListItem::new(Span::styled(text, style))
                })
                .collect();

            let block = Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(" Fuse Source ", theme.title));

            let list = List::new(items).block(block);
            frame.render_widget(list, list_area);
        }

        // Abort confirmation
        if self.confirm_abort {
            let msg_area = Rect {
                x: area.x + 2,
                y: area.y + area.height.saturating_sub(2),
                width: area.width.saturating_sub(4),
                height: 1,
            };
            let msg = Paragraph::new(Span::styled(
                "Abort merge? y:yes any:cancel",
                theme.warning,
            ));
            frame.render_widget(msg, msg_area);
        }

        // Help
        if area.height > 2 && !self.confirm_abort {
            let help_area = Rect {
                x: area.x,
                y: area.y + area.height - 1,
                width: area.width,
                height: 1,
            };
            let help_text = if self.merge_in_progress {
                " a:abort r:refresh"
            } else {
                " Enter/f:fuse s:strategy a:abort r:refresh"
            };
            let help = Paragraph::new(Span::styled(help_text, theme.dim));
            frame.render_widget(help, help_area);
        }
    }

    fn load_data(&mut self, ctx: &AppContext) {
        let current = ctx.repo.current_timeline().unwrap_or_default();

        self.timelines = ctx
            .repo
            .list_timelines()
            .unwrap_or_default()
            .into_iter()
            .filter(|(name, _)| *name != current)
            .map(|(name, _)| (name, false))
            .collect();

        self.timelines.sort_by(|a, b| a.0.cmp(&b.0));

        // Check merge state
        if let Ok(Some(state)) = ctx.repo.load_merge_state() {
            self.merge_in_progress = true;
            self.merge_conflicts = state.conflicts;
        } else {
            self.merge_in_progress = false;
            self.merge_conflicts.clear();
        }

        if self.cursor >= self.timelines.len() && !self.timelines.is_empty() {
            self.cursor = self.timelines.len() - 1;
        }
    }

    fn short_help(&self) -> &str {
        "Enter/f:fuse s:strategy a:abort"
    }

    fn has_active_input(&self) -> bool {
        self.confirm_abort
    }
}
