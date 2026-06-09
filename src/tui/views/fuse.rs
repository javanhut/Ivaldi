//! Fuse (merge) tab — merge timelines together.

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::fuse::Strategy;
use crate::hash::B3Hash;
use crate::tui::resolver::{ConflictItem, Resolution, CHOICES};
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

/// In-flight interactive resolution state, held while the resolver modal is up.
struct PendingFuse {
    source_name: String,
    target_name: String,
    ours: BTreeMap<String, B3Hash>,
    theirs: BTreeMap<String, B3Hash>,
    /// Auto-resolved (non-conflicting) files.
    merged_files: BTreeMap<String, B3Hash>,
    conflicts: Vec<ConflictItem>,
    /// Parallel to `conflicts`.
    resolutions: Vec<Option<Resolution>>,
    /// Conflict currently being decided.
    current: usize,
    /// Highlighted choice within CHOICES.
    cursor: usize,
}

pub struct FuseView {
    timelines: Vec<(String, bool)>, // name, is_current — excludes current
    cursor: usize,
    strategy_idx: usize,
    merge_in_progress: bool,
    merge_conflicts: Vec<String>,
    confirm_abort: bool,
    pending: Option<PendingFuse>,
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
            pending: None,
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

        // Resolve head indices for both timelines.
        let our_idx = match ctx.repo.get_timeline_head(&current) {
            Ok(Some(idx)) => idx,
            Ok(None) => return Action::Error("Current timeline has no commits".into()),
            Err(e) => return Action::Error(format!("Failed: {}", e)),
        };
        let their_idx = match ctx.repo.get_timeline_head(&source_name) {
            Ok(Some(idx)) => idx,
            Ok(None) => {
                return Action::Error(format!("Timeline '{}' has no commits", source_name));
            }
            Err(e) => return Action::Error(format!("Failed: {}", e)),
        };

        let our_tree = match ctx.repo.get_leaf(our_idx) {
            Ok(Some(l)) => l.tree_root,
            _ => return Action::Error("Current timeline has no commits".into()),
        };
        let their_tree = match ctx.repo.get_leaf(their_idx) {
            Ok(Some(l)) => l.tree_root,
            _ => return Action::Error(format!("Timeline '{}' has no commits", source_name)),
        };

        // Real LCA-based merge base (empty only for unrelated histories),
        // mirroring the CLI. Using an empty base here would flag every
        // differing file as a spurious conflict.
        let base = match ctx.repo.merge_base(our_idx, their_idx) {
            Ok(Some(base_idx)) => match ctx.repo.get_leaf(base_idx) {
                Ok(Some(l)) => self.load_tree_map(ctx, l.tree_root),
                _ => BTreeMap::new(),
            },
            Ok(None) => BTreeMap::new(),
            Err(e) => return Action::Error(format!("Merge base failed: {}", e)),
        };

        let ours = self.load_tree_map(ctx, our_tree);
        let theirs = self.load_tree_map(ctx, their_tree);

        let store = crate::fsmerkle::FsStore::new(&ctx.repo.cas);
        let result = crate::fuse::FuseEngine::fuse(
            &store,
            &base,
            &ours,
            &theirs,
            self.current_strategy(),
        );

        if result.success {
            return self.commit_merged(ctx, &result.merged_files, &source_name, &current);
        }

        // Conflicts: open the interactive resolver modal.
        let conflicts: Vec<ConflictItem> = result
            .conflicts
            .iter()
            .map(|c| ConflictItem {
                path: c.path.clone(),
                description: conflict_description(c),
            })
            .collect();
        let n = conflicts.len();
        self.pending = Some(PendingFuse {
            source_name,
            target_name: current,
            ours,
            theirs,
            merged_files: result.merged_files,
            conflicts,
            resolutions: vec![None; n],
            current: 0,
            cursor: 0,
        });
        Action::Consumed
    }

    /// Build a tree from `merged` and commit it as a fuse seal.
    fn commit_merged(
        &mut self,
        ctx: &mut AppContext,
        merged: &BTreeMap<String, B3Hash>,
        source: &str,
        target: &str,
    ) -> Action {
        let config = crate::config::load_config(&ctx.ivaldi_dir);
        let author = config
            .author()
            .unwrap_or_else(|| "unknown <unknown>".into());
        let msg = format!("Fuse {} into {}", source, target);

        let store = crate::fsmerkle::FsStore::new(&ctx.repo.cas);
        match store.build_tree_from_hash_map(merged) {
            Ok(tree_root) => match ctx.repo.commit(tree_root, &author, &msg) {
                Ok(cr) => {
                    let _ = ctx.repo.clear_merge_state();
                    self.merge_in_progress = false;
                    self.merge_conflicts.clear();
                    Action::Success(format!("Fuse complete: {}", cr.seal_name))
                }
                Err(e) => Action::Error(format!("Commit failed: {}", e)),
            },
            Err(e) => Action::Error(format!("Tree build failed: {}", e)),
        }
    }

    // --- resolver modal -------------------------------------------------

    fn handle_resolver_event(&mut self, event: &KeyEvent, ctx: &mut AppContext) -> Action {
        match event.code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('a') => {
                return self.abort_resolver(ctx);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(p) = self.pending.as_mut() {
                    if p.cursor > 0 {
                        p.cursor -= 1;
                    }
                }
                return Action::Consumed;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(p) = self.pending.as_mut() {
                    if p.cursor + 1 < CHOICES.len() {
                        p.cursor += 1;
                    }
                }
                return Action::Consumed;
            }
            KeyCode::Char('1') => self.resolve_current(Resolution::Ours),
            KeyCode::Char('2') => self.resolve_current(Resolution::Theirs),
            KeyCode::Char('3') => self.resolve_current(Resolution::Both),
            KeyCode::Char('4') => self.resolve_current(Resolution::Skip),
            KeyCode::Enter => {
                if let Some(choice) = self.pending.as_ref().map(|p| CHOICES[p.cursor].1) {
                    self.resolve_current(choice);
                }
            }
            _ => return Action::Consumed,
        }

        // Apply once every conflict has a decision.
        let all_done = self
            .pending
            .as_ref()
            .map(|p| p.resolutions.iter().all(|r| r.is_some()))
            .unwrap_or(false);
        if all_done {
            return self.apply_pending(ctx);
        }
        Action::Consumed
    }

    fn resolve_current(&mut self, res: Resolution) {
        if let Some(p) = self.pending.as_mut() {
            if p.conflicts.is_empty() {
                return;
            }
            p.resolutions[p.current] = Some(res);
            // Advance to the next undecided conflict.
            for i in 0..p.conflicts.len() {
                let idx = (p.current + 1 + i) % p.conflicts.len();
                if p.resolutions[idx].is_none() {
                    p.current = idx;
                    p.cursor = 0;
                    return;
                }
            }
        }
    }

    fn abort_resolver(&mut self, ctx: &mut AppContext) -> Action {
        let Some(p) = self.pending.take() else {
            return Action::Consumed;
        };
        let conflicts: Vec<String> = p.conflicts.iter().map(|c| c.path.clone()).collect();
        let state = crate::repo::MergeState {
            source_timeline: p.source_name,
            target_timeline: p.target_name,
            strategy: self.current_strategy().to_string(),
            conflicts: conflicts.clone(),
        };
        match ctx.repo.save_merge_state(&state) {
            Ok(()) => {
                self.merge_in_progress = true;
                self.merge_conflicts = conflicts;
                Action::Error("Resolution cancelled; merge left in progress.".into())
            }
            Err(e) => Action::Error(format!("Failed to save merge state: {}", e)),
        }
    }

    fn apply_pending(&mut self, ctx: &mut AppContext) -> Action {
        let Some(p) = self.pending.take() else {
            return Action::Consumed;
        };
        let resolutions: Vec<(String, Resolution)> = p
            .conflicts
            .iter()
            .zip(p.resolutions.iter())
            .filter_map(|(c, r)| r.map(|res| (c.path.clone(), res)))
            .collect();

        let store = crate::fsmerkle::FsStore::new(&ctx.repo.cas);
        let (final_map, skipped) =
            apply_resolutions(&store, &p.merged_files, &p.ours, &p.theirs, &resolutions);

        if !skipped.is_empty() {
            // Skipped files stay unresolved — do not commit; keep the merge
            // in progress with only those paths outstanding.
            let state = crate::repo::MergeState {
                source_timeline: p.source_name.clone(),
                target_timeline: p.target_name.clone(),
                strategy: self.current_strategy().to_string(),
                conflicts: skipped.clone(),
            };
            let _ = ctx.repo.save_merge_state(&state);
            self.merge_in_progress = true;
            self.merge_conflicts = skipped.clone();
            return Action::Error(format!(
                "{} file(s) skipped; merge left in progress.",
                skipped.len()
            ));
        }

        self.commit_merged(ctx, &final_map, &p.source_name, &p.target_name)
    }

    fn load_tree_map(
        &self,
        ctx: &AppContext,
        tree_hash: B3Hash,
    ) -> BTreeMap<String, B3Hash> {
        let store = crate::fsmerkle::FsStore::new(&ctx.repo.cas);
        let mut map = BTreeMap::new();
        let _ = Self::collect_tree(&store, tree_hash, "", &mut map);
        map
    }

    fn collect_tree(
        store: &crate::fsmerkle::FsStore<'_>,
        hash: B3Hash,
        prefix: &str,
        map: &mut BTreeMap<String, B3Hash>,
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

    fn render_resolver(&self, frame: &mut Frame, area: Rect, theme: &Theme, p: &PendingFuse) {
        let resolved = p.resolutions.iter().filter(|r| r.is_some()).count();
        let total = p.conflicts.len();

        // Header.
        let header = Paragraph::new(Span::styled(
            format!(" Resolve conflicts — {}/{} decided", resolved, total),
            theme.title,
        ))
        .block(Block::default().borders(Borders::ALL).title(" Fuse Resolver "));
        let header_area = Rect { height: 3.min(area.height), ..area };
        frame.render_widget(header, header_area);

        if let Some(conflict) = p.conflicts.get(p.current) {
            let status = match p.resolutions[p.current] {
                Some(Resolution::Ours) => " [→ OURS]",
                Some(Resolution::Theirs) => " [→ THEIRS]",
                Some(Resolution::Both) => " [→ BOTH]",
                Some(Resolution::Skip) => " [→ SKIPPED]",
                None => "",
            };
            let conflict_area = Rect {
                x: area.x,
                y: area.y + 3,
                width: area.width,
                height: 3.min(area.height.saturating_sub(3)),
            };
            let text = Paragraph::new(vec![
                Line::from(Span::styled(
                    format!(
                        " Conflict {} of {}: {}{}",
                        p.current + 1,
                        total,
                        conflict.path,
                        status
                    ),
                    theme.warning,
                )),
                Line::from(Span::styled(format!(" {}", conflict.description), theme.dim)),
            ])
            .block(Block::default().borders(Borders::ALL));
            frame.render_widget(text, conflict_area);
        }

        // Choices.
        let choices_y = area.y + 6;
        let choices_area = Rect {
            x: area.x,
            y: choices_y,
            width: area.width,
            height: area.height.saturating_sub(9),
        };
        let items: Vec<ListItem> = CHOICES
            .iter()
            .enumerate()
            .map(|(i, (label, _))| {
                let marker = if i == p.cursor { "→" } else { " " };
                let text = format!("{} [{}] {}", marker, i + 1, label);
                let style = if i == p.cursor {
                    theme.cursor
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Span::styled(text, style))
            })
            .collect();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Choose resolution "));
        frame.render_widget(list, choices_area);

        // Footer help.
        if area.height > 1 {
            let help_area = Rect {
                x: area.x,
                y: area.y + area.height - 1,
                width: area.width,
                height: 1,
            };
            let help = Paragraph::new(Span::styled(
                " ↑/↓ choose • 1-4 pick • Enter confirm • a/q abort",
                theme.dim,
            ));
            frame.render_widget(help, help_area);
        }
    }
}

/// Apply per-file resolutions on top of the auto-merged map.
///
/// Returns the final `path → hash` map and the list of paths the user chose to
/// skip (left unresolved). Skipped paths are omitted from the map and signal the
/// caller not to commit.
pub(crate) fn apply_resolutions(
    store: &crate::fsmerkle::FsStore<'_>,
    merged: &BTreeMap<String, B3Hash>,
    ours: &BTreeMap<String, B3Hash>,
    theirs: &BTreeMap<String, B3Hash>,
    resolutions: &[(String, Resolution)],
) -> (BTreeMap<String, B3Hash>, Vec<String>) {
    let mut final_map = merged.clone();
    let mut skipped = Vec::new();

    for (path, res) in resolutions {
        match res {
            Resolution::Ours => match ours.get(path) {
                Some(h) => {
                    final_map.insert(path.clone(), *h);
                }
                None => {
                    final_map.remove(path);
                }
            },
            Resolution::Theirs => match theirs.get(path) {
                Some(h) => {
                    final_map.insert(path.clone(), *h);
                }
                None => {
                    final_map.remove(path);
                }
            },
            Resolution::Both => match (ours.get(path), theirs.get(path)) {
                (Some(o), Some(t)) => {
                    let h = crate::fuse::concat_blobs(store, o, t).unwrap_or(*o);
                    final_map.insert(path.clone(), h);
                }
                (Some(h), None) | (None, Some(h)) => {
                    final_map.insert(path.clone(), *h);
                }
                (None, None) => {
                    final_map.remove(path);
                }
            },
            Resolution::Skip => skipped.push(path.clone()),
        }
    }

    (final_map, skipped)
}

/// Human-readable summary of a file conflict.
fn conflict_description(c: &crate::fuse::Conflict) -> String {
    match (c.base.is_some(), c.ours.is_some(), c.theirs.is_some()) {
        (_, true, true) => "modified on both sides".into(),
        (true, true, false) => "modified here, deleted on the other side".into(),
        (true, false, true) => "deleted here, modified on the other side".into(),
        _ => "conflicting change".into(),
    }
}

impl TabView for FuseView {
    fn handle_event(&mut self, event: &KeyEvent, ctx: &mut AppContext) -> Action {
        // Interactive resolver modal takes precedence.
        if self.pending.is_some() {
            return self.handle_resolver_event(event, ctx);
        }

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
        // Resolver modal replaces the normal view.
        if let Some(p) = self.pending.as_ref() {
            self.render_resolver(frame, area, theme, p);
            return;
        }

        // Merge in progress banner
        if self.merge_in_progress {
            let banner_area = Rect {
                height: 2.min(area.height),
                ..area
            };
            let banner = Paragraph::new(vec![
                Line::from(Span::styled("MERGE IN PROGRESS", theme.warning)),
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
            Span::styled(format!("{}", self.current_strategy()), theme.brand),
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
            let msg = Paragraph::new(Span::styled("No other timelines to fuse", theme.dim));
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
            let msg = Paragraph::new(Span::styled("Abort merge? y:yes any:cancel", theme.warning));
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
        self.confirm_abort || self.pending.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store() -> (tempfile::TempDir, crate::cas::FileCas) {
        let dir = tempfile::tempdir().unwrap();
        let cas = crate::cas::FileCas::new(dir.path().join("objects")).unwrap();
        (dir, cas)
    }

    #[test]
    fn apply_resolutions_covers_all_choices() {
        let (_dir, cas) = tmp_store();
        let store = crate::fsmerkle::FsStore::new(&cas);

        // Auto-merged, non-conflicting file carries through untouched.
        let (clean, _) = store.put_blob(b"clean").unwrap();
        let merged: BTreeMap<String, B3Hash> = [("clean.txt".to_string(), clean)].into();

        let put = |c: &[u8]| store.put_blob(c).unwrap().0;
        let ours: BTreeMap<String, B3Hash> = [
            ("a.txt".to_string(), put(b"OURS_A")),
            ("b.txt".to_string(), put(b"OURS_B")),
            ("c.txt".to_string(), put(b"OURS_C")),
            ("d.txt".to_string(), put(b"OURS_D")),
        ]
        .into();
        let theirs: BTreeMap<String, B3Hash> = [
            ("a.txt".to_string(), put(b"THEIRS_A")),
            ("b.txt".to_string(), put(b"THEIRS_B")),
            ("c.txt".to_string(), put(b"THEIRS_C")),
            ("d.txt".to_string(), put(b"THEIRS_D")),
        ]
        .into();

        let resolutions = vec![
            ("a.txt".to_string(), Resolution::Ours),
            ("b.txt".to_string(), Resolution::Theirs),
            ("c.txt".to_string(), Resolution::Both),
            ("d.txt".to_string(), Resolution::Skip),
        ];

        let (final_map, skipped) =
            apply_resolutions(&store, &merged, &ours, &theirs, &resolutions);

        // Skip leaves the file unresolved and out of the committed map.
        assert_eq!(skipped, vec!["d.txt".to_string()]);
        assert!(!final_map.contains_key("d.txt"));

        // Clean auto-merge survives.
        assert_eq!(final_map["clean.txt"], clean);

        let load = |h: B3Hash| store.load_blob(h).unwrap().1;
        assert_eq!(load(final_map["a.txt"]), b"OURS_A");
        assert_eq!(load(final_map["b.txt"]), b"THEIRS_B");
        // Both concatenates ours then theirs.
        assert_eq!(load(final_map["c.txt"]), b"OURS_CTHEIRS_C");
    }
}
