//! Fuse (merge) tab — merge timelines together.
//!
//! The fuse itself is [`crate::fuse_op`], the same as the CLI's. What this
//! view adds is a way of asking: collisions the engine could not settle are
//! laid out as [`Questions`] and put to the user one at a time as a modal,
//! each showing both versions. Nothing is written until the last one is
//! answered, so backing out of the modal leaves the repository untouched.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::fuse::{Resolution, Strategy};
use crate::fuse_op::{FusePlan, Planned};
use crate::resolve::{Answer, Question, Questions, Side, WholeFile};
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

/// A planned fuse waiting on answers, held while the modal is up.
struct PendingFuse {
    plan: FusePlan,
    questions: Questions,
    /// Question on screen.
    current: usize,
    /// Highlighted choice.
    cursor: usize,
}

impl PendingFuse {
    /// The answers on offer for the question on screen, in hotkey order.
    fn choices(&self) -> Vec<(String, Answer)> {
        let (mine, theirs) = (&self.plan.target, &self.plan.source);
        match self.questions.get(self.current) {
            Some(Question::Region { .. }) => vec![
                (format!("Mine ({mine})"), Answer::Region(Resolution::Ours)),
                (
                    format!("Theirs ({theirs})"),
                    Answer::Region(Resolution::Theirs),
                ),
                (
                    "Both — mine, then theirs".into(),
                    Answer::Region(Resolution::Both),
                ),
            ],
            Some(Question::WholeFile { why, .. }) => {
                let (m, t) = match why {
                    WholeFile::Binary => ("Keep my version", "Take their version"),
                    WholeFile::DeletedByTheirs => {
                        ("Keep my changed file", "Delete it, as they did")
                    }
                    WholeFile::DeletedByMine => ("Keep it deleted", "Take their changed file"),
                };
                vec![
                    (format!("{m} ({mine})"), Answer::WholeFile(Side::Mine)),
                    (format!("{t} ({theirs})"), Answer::WholeFile(Side::Theirs)),
                ]
            }
            None => Vec::new(),
        }
    }
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

impl Default for FuseView {
    fn default() -> Self {
        Self::new()
    }
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
        let source = match self.timelines.get(self.cursor) {
            Some((name, _)) => name.clone(),
            None => return Action::Error("No timeline selected".into()),
        };

        // An earlier fuse that died still owing set-aside work is settled
        // before another is started.
        if let Err(e) = crate::fuse_op::finish_interrupted(&ctx.repo, None) {
            return Action::Error(format!("Fuse failed: {e}"));
        }

        let plan = match crate::fuse_op::plan(&ctx.repo, &source, self.current_strategy()) {
            Ok(Planned::Plan(plan)) => plan,
            Ok(Planned::AlreadyFused) => {
                return Action::Success(format!("'{source}' is already fused — nothing to do"));
            }
            Err(e) => return Action::Error(format!("Fuse failed: {e}")),
        };
        if plan.conflicts.is_empty() {
            return self.complete(ctx, &plan);
        }

        // True collisions: ask, one at a time, before anything is written.
        let store = crate::fsmerkle::FsStore::new(&ctx.repo.cas);
        match Questions::new(&store, &plan.conflicts) {
            Ok(questions) => {
                self.pending = Some(PendingFuse {
                    plan,
                    questions,
                    current: 0,
                    cursor: 0,
                });
                Action::Consumed
            }
            Err(e) => Action::Error(format!("Fuse failed: {e}")),
        }
    }

    /// Seal, materialize, and carry uncommitted work through — everything the
    /// CLI's fuse does, because it is the same code.
    fn complete(&mut self, ctx: &mut AppContext, plan: &FusePlan) -> Action {
        // Collisions between the fuse and *uncommitted* work are not asked
        // about here; with no resolver they get conflict markers in the
        // (still uncommitted) file, and the message says so.
        let fused = match crate::fuse_op::complete(&mut ctx.repo, plan, None) {
            Ok(fused) => fused,
            Err(e) => return Action::Error(format!("Fuse failed: {e}")),
        };
        self.merge_in_progress = false;
        self.merge_conflicts.clear();

        let mut msg = format!("Fuse complete: {}", fused.seal.seal_name);
        if let Some(report) = &fused.carry {
            let attention = report.attention().count();
            msg.push_str(&format!(
                " — {} uncommitted change(s) carried through",
                report.outcomes.len()
            ));
            if attention > 0 {
                msg.push_str(&format!(", {attention} need a look (see Status)"));
            }
        }
        msg.push_str(". Undo: 'ivaldi oops'");
        Action::Success(msg)
    }

    // --- question modal ---------------------------------------------------

    fn handle_question_event(&mut self, event: &KeyEvent, ctx: &mut AppContext) -> Action {
        let Some(p) = self.pending.as_mut() else {
            return Action::Consumed;
        };
        let choices = p.choices();
        let picked = match event.code {
            // Nothing has been written, so backing out is just forgetting.
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('a') => {
                self.pending = None;
                return Action::Success("Fuse cancelled — nothing was changed".into());
            }
            KeyCode::Up | KeyCode::Char('k') => {
                p.cursor = p.cursor.saturating_sub(1);
                return Action::Consumed;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if p.cursor + 1 < choices.len() {
                    p.cursor += 1;
                }
                return Action::Consumed;
            }
            KeyCode::Char(c @ '1'..='9') => (c as usize) - ('1' as usize),
            KeyCode::Enter => p.cursor,
            _ => return Action::Consumed,
        };
        let Some((_, answer)) = choices.into_iter().nth(picked) else {
            return Action::Consumed;
        };
        p.questions.answer(p.current, answer);

        match p.questions.next_unanswered(p.current) {
            Some(next) => {
                p.current = next;
                p.cursor = 0;
                Action::Consumed
            }
            None => {
                let Some(mut p) = self.pending.take() else {
                    return Action::Consumed;
                };
                let store = crate::fsmerkle::FsStore::new(&ctx.repo.cas);
                if let Err(e) = p.questions.apply(&store, &mut p.plan.merged_files) {
                    return Action::Error(format!("Fuse failed: {e}"));
                }
                self.complete(ctx, &p.plan)
            }
        }
    }

    fn render_question(&self, frame: &mut Frame, area: Rect, theme: &Theme, p: &PendingFuse) {
        let total = p.questions.len();
        let answered = (0..total).filter(|&i| p.questions.answered(i)).count();
        let choices = p.choices();

        let header_h = 3.min(area.height);
        let choices_h = (choices.len() as u16 + 2).min(area.height.saturating_sub(header_h));
        let help_h = 1.min(area.height.saturating_sub(header_h + choices_h));
        let body_h = area.height.saturating_sub(header_h + choices_h + help_h);

        let header = Paragraph::new(Span::styled(
            format!(
                " Fusing {} into {} — {answered} of {total} collision(s) settled. \
                 Nothing is changed until the last one.",
                p.plan.source, p.plan.target
            ),
            theme.title,
        ))
        .block(Block::default().borders(Borders::ALL).title(" Fuse "));
        frame.render_widget(
            header,
            Rect {
                height: header_h,
                ..area
            },
        );

        // The question: both versions, with a little of what surrounds them.
        let mine = Style::default().fg(Color::Green);
        let theirs = Style::default().fg(Color::Cyan);
        let mut lines: Vec<Line> = Vec::new();
        match p.questions.get(p.current) {
            Some(Question::Region {
                path,
                index,
                total: in_file,
                collision,
                before,
                after,
            }) => {
                lines.push(Line::from(Span::styled(
                    format!(
                        " {path} — collision {index} of {in_file}, around line {}",
                        collision.ours_line
                    ),
                    theme.warning,
                )));
                // Both sides get the same share of the room that is left once
                // the fixed lines are in; a long side says how much it hid.
                let fixed = 3 + before.len() + after.len();
                let room = (body_h.saturating_sub(2) as usize).saturating_sub(fixed);
                let per_side = (room / 2).max(1);
                let side =
                    |lines: &mut Vec<Line>, label: String, content: &[String], style: Style| {
                        lines.push(Line::from(Span::styled(format!(" {label}:"), style)));
                        if content.is_empty() {
                            lines.push(Line::from(Span::styled(
                                "   (these lines deleted)",
                                theme.dim,
                            )));
                        }
                        for l in content.iter().take(per_side) {
                            lines.push(Line::from(vec![
                                Span::styled(" | ", style),
                                Span::raw(l.clone()),
                            ]));
                        }
                        if content.len() > per_side {
                            lines.push(Line::from(Span::styled(
                                format!("   … {} more line(s)", content.len() - per_side),
                                theme.dim,
                            )));
                        }
                    };
                for l in before {
                    lines.push(Line::from(Span::styled(format!("   {l}"), theme.dim)));
                }
                side(
                    &mut lines,
                    format!("mine ({})", p.plan.target),
                    &collision.ours,
                    mine,
                );
                side(
                    &mut lines,
                    format!("theirs ({})", p.plan.source),
                    &collision.theirs,
                    theirs,
                );
                for l in after {
                    lines.push(Line::from(Span::styled(format!("   {l}"), theme.dim)));
                }
            }
            Some(Question::WholeFile { path, why }) => {
                lines.push(Line::from(Span::styled(format!(" {path}"), theme.warning)));
                lines.push(Line::from(Span::styled(
                    match why {
                        WholeFile::Binary => {
                            " Binary, changed on both sides — it can only be one or the other."
                        }
                        WholeFile::DeletedByTheirs => " Changed here, deleted there.",
                        WholeFile::DeletedByMine => " Deleted here, changed there.",
                    },
                    theme.dim,
                )));
            }
            None => {}
        }
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Question {} of {total} ", p.current + 1)),
            ),
            Rect {
                y: area.y + header_h,
                height: body_h,
                ..area
            },
        );

        let items: Vec<ListItem> = choices
            .iter()
            .enumerate()
            .map(|(i, (label, _))| {
                let marker = if i == p.cursor { "→" } else { " " };
                let style = if i == p.cursor {
                    theme.cursor
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Span::styled(format!("{marker} [{}] {label}", i + 1), style))
            })
            .collect();
        frame.render_widget(
            List::new(items).block(Block::default().borders(Borders::ALL).title(" Keep ")),
            Rect {
                y: area.y + header_h + body_h,
                height: choices_h,
                ..area
            },
        );

        if help_h > 0 {
            let help = Paragraph::new(Span::styled(
                " ↑/↓ choose • 1-3 pick • Enter confirm • q cancel (nothing changed) • \
                 neither fits? cancel, and 'ivaldi fuse' in a terminal can edit the lines",
                theme.dim,
            ));
            frame.render_widget(
                help,
                Rect {
                    y: area.y + area.height - 1,
                    height: 1,
                    ..area
                },
            );
        }
    }
}

impl TabView for FuseView {
    fn handle_event(&mut self, event: &KeyEvent, ctx: &mut AppContext) -> Action {
        // The question modal takes precedence.
        if self.pending.is_some() {
            return self.handle_question_event(event, ctx);
        }

        // Abort confirmation
        if self.confirm_abort {
            match event.code {
                KeyCode::Char('y') => {
                    self.confirm_abort = false;
                    // Also gives back any uncommitted work the fuse had set
                    // aside, and clears conflict markers out of the files.
                    match crate::fuse_op::abort(&ctx.repo) {
                        Ok(restored) => {
                            self.merge_in_progress = false;
                            self.merge_conflicts.clear();
                            Action::Success(if restored {
                                "Merge aborted; your uncommitted changes are back".into()
                            } else {
                                "Merge aborted".into()
                            })
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
        // The question modal replaces the normal view.
        if let Some(p) = self.pending.as_ref() {
            self.render_question(frame, area, theme, p);
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
    use crossterm::event::KeyModifiers;
    use std::collections::BTreeMap;

    fn doc(edits: &[(usize, &str)]) -> String {
        (1..=30)
            .map(|n| match edits.iter().find(|(line, _)| *line == n) {
                Some((_, text)) => format!("{text}\n"),
                None => format!("line {n}\n"),
            })
            .collect()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn seal(repo: &mut crate::repo::Repo, files: &[(&str, &str)], message: &str) {
        let files: BTreeMap<String, Vec<u8>> = files
            .iter()
            .map(|(path, content)| (path.to_string(), content.as_bytes().to_vec()))
            .collect();
        let tree = crate::fsmerkle::FsStore::new(&repo.cas)
            .build_tree_from_map(&files)
            .unwrap();
        repo.commit(tree, "author", message).unwrap();
    }

    /// `main` (current) and `feature` collide on lines 3 and 25 of `a.txt`;
    /// each also has an edit of its own — kept well clear of those lines,
    /// because the engine folds any edit within a few lines of a collision
    /// into it rather than splice around it. The working directory matches
    /// `main`.
    fn colliding(dir: &std::path::Path) -> AppContext {
        crate::forge::forge(dir).unwrap();
        let mut repo = crate::repo::Repo::open(dir).unwrap();
        seal(&mut repo, &[("a.txt", &doc(&[]))], "base");
        repo.create_timeline("feature", None).unwrap();
        repo.switch_timeline("feature").unwrap();
        seal(
            &mut repo,
            &[(
                "a.txt",
                &doc(&[(3, "FEATURE 3"), (25, "FEATURE 25"), (17, "FEATURE ONLY")]),
            )],
            "feature edit",
        );
        repo.switch_timeline("main").unwrap();
        let mine = doc(&[(3, "MAIN 3"), (25, "MAIN 25"), (11, "MAIN ONLY")]);
        seal(&mut repo, &[("a.txt", &mine)], "main edit");
        std::fs::write(dir.join("a.txt"), &mine).unwrap();
        AppContext {
            work_dir: dir.to_path_buf(),
            ivaldi_dir: dir.join(".ivaldi"),
            repo,
        }
    }

    fn view_on_feature(ctx: &AppContext) -> FuseView {
        let mut view = FuseView::new();
        view.load_data(ctx);
        assert_eq!(view.timelines, [("feature".to_string(), false)]);
        view
    }

    #[test]
    fn answers_each_collision_then_seals_a_real_merge_in_one_go() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = colliding(dir.path());
        let mut view = view_on_feature(&ctx);
        let head_before = ctx.repo.get_timeline_head("main").unwrap();

        // Fusing opens the modal on the first collision; nothing is written.
        assert!(matches!(
            view.handle_event(&key(KeyCode::Enter), &mut ctx),
            Action::Consumed
        ));
        let pending = view.pending.as_ref().expect("collisions open the modal");
        assert_eq!(pending.questions.len(), 2);
        assert!(view.has_active_input());
        assert_eq!(ctx.repo.get_timeline_head("main").unwrap(), head_before);

        // Theirs for the first, both for the second.
        assert!(matches!(
            view.handle_event(&key(KeyCode::Char('2')), &mut ctx),
            Action::Consumed
        ));
        assert_eq!(view.pending.as_ref().unwrap().current, 1);
        let done = view.handle_event(&key(KeyCode::Char('3')), &mut ctx);
        let Action::Success(message) = done else {
            panic!("the last answer completes the fuse");
        };
        assert!(message.contains("ivaldi oops"), "{message}");
        assert!(view.pending.is_none());

        // The merged tree is in the working directory — collisions as
        // answered, and each side's own edit kept.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            doc(&[
                (3, "FEATURE 3"),
                (11, "MAIN ONLY"),
                (17, "FEATURE ONLY"),
                (25, "MAIN 25\nFEATURE 25")
            ])
        );
        // It is a merge: the source head is a parent, so fusing again is a no-op.
        let head = ctx.repo.get_timeline_head("main").unwrap().unwrap();
        let leaf = ctx.repo.get_leaf(head).unwrap().unwrap();
        assert!(leaf.is_merge());
        assert!(matches!(
            view.handle_event(&key(KeyCode::Enter), &mut ctx),
            Action::Success(m) if m.contains("already fused")
        ));
        // And it can be taken back.
        let snapshot = crate::snapshot::SnapshotManager::new(&ctx.ivaldi_dir)
            .latest()
            .unwrap()
            .expect("the fuse is snapshotted for oops");
        assert_eq!(snapshot.command, "fuse feature");
        assert_eq!(snapshot.head, head_before);
    }

    #[test]
    fn cancelling_the_modal_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = colliding(dir.path());
        let mut view = view_on_feature(&ctx);
        let head_before = ctx.repo.get_timeline_head("main").unwrap();
        let file_before = std::fs::read_to_string(dir.path().join("a.txt")).unwrap();

        view.handle_event(&key(KeyCode::Enter), &mut ctx);
        view.handle_event(&key(KeyCode::Char('1')), &mut ctx); // one answer in
        let cancelled = view.handle_event(&key(KeyCode::Esc), &mut ctx);
        assert!(matches!(cancelled, Action::Success(m) if m.contains("nothing was changed")));

        assert!(view.pending.is_none());
        assert_eq!(ctx.repo.get_timeline_head("main").unwrap(), head_before);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            file_before
        );
        assert!(!ctx.repo.has_merge_in_progress(), "no merge is left open");
        assert!(
            crate::snapshot::SnapshotManager::new(&ctx.ivaldi_dir)
                .latest()
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn uncommitted_work_is_carried_through_a_tui_fuse() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = colliding(dir.path());
        let mut view = view_on_feature(&ctx);
        // An unsealed edit far from everything else, and a new file.
        let dirty = doc(&[
            (3, "MAIN 3"),
            (25, "MAIN 25"),
            (11, "MAIN ONLY"),
            (30, "UNSEALED"),
        ]);
        std::fs::write(dir.path().join("a.txt"), &dirty).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "scratch\n").unwrap();

        view.handle_event(&key(KeyCode::Enter), &mut ctx);
        view.handle_event(&key(KeyCode::Char('1')), &mut ctx);
        let done = view.handle_event(&key(KeyCode::Char('1')), &mut ctx);
        assert!(
            matches!(&done, Action::Success(m) if m.contains("2 uncommitted change(s) carried")),
            "unexpected result"
        );

        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            doc(&[
                (3, "MAIN 3"),
                (11, "MAIN ONLY"),
                (17, "FEATURE ONLY"),
                (25, "MAIN 25"),
                (30, "UNSEALED")
            ])
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("notes.txt")).unwrap(),
            "scratch\n"
        );
    }

    #[test]
    fn renders_both_sides_of_the_collision() {
        use ratatui::{Terminal, backend::TestBackend};
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = colliding(dir.path());
        let mut view = view_on_feature(&ctx);
        view.handle_event(&key(KeyCode::Enter), &mut ctx);

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| view.render(frame, frame.area(), &Theme::default_theme()))
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        for expected in [
            "a.txt — collision 1 of 2, around line 3",
            "mine (main)",
            "MAIN 3",
            "theirs (feature)",
            "FEATURE 3",
            "[3] Both",
            "0 of 2 collision(s) settled",
        ] {
            assert!(screen.contains(expected), "missing {expected:?}");
        }
    }
}
