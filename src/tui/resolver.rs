//! Interactive conflict resolver for fuse operations.
//!
//! Presents each conflicted file and lets the user choose:
//! 1. Keep ours (target timeline)
//! 2. Keep theirs (source timeline)
//! 3. Keep both
//! 4. Skip
//! 5. Abort merge

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::prelude::*;
use ratatui::widgets::*;

/// A conflict to resolve.
#[derive(Debug, Clone)]
pub struct ConflictItem {
    pub path: String,
    pub description: String,
}

/// Resolution choice for a single file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resolution {
    Ours,
    Theirs,
    Both,
    Skip,
}

/// Result of the resolver.
#[derive(Debug)]
pub enum ResolverResult {
    /// All conflicts resolved.
    Resolved(Vec<(String, Resolution)>),
    /// User aborted.
    Aborted,
}

struct ResolverState {
    conflicts: Vec<ConflictItem>,
    current: usize,
    resolutions: Vec<Option<Resolution>>,
    cursor: usize,
}

const CHOICES: &[(&str, Resolution)] = &[
    ("Keep OURS (target timeline)", Resolution::Ours),
    ("Keep THEIRS (source timeline)", Resolution::Theirs),
    ("Keep BOTH (concatenate)", Resolution::Both),
    ("Skip this file", Resolution::Skip),
];

/// Run the interactive conflict resolver.
pub fn run_resolver(conflicts: Vec<ConflictItem>) -> std::io::Result<ResolverResult> {
    if conflicts.is_empty() {
        return Ok(ResolverResult::Resolved(Vec::new()));
    }

    let count = conflicts.len();
    let mut terminal = super::init_terminal()?;
    let mut state = ResolverState {
        resolutions: vec![None; count],
        conflicts,
        current: 0,
        cursor: 0,
    };

    let result = loop {
        terminal.draw(|frame| draw_resolver(frame, &state))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break ResolverResult::Aborted,
                KeyCode::Up => {
                    if state.cursor > 0 {
                        state.cursor -= 1;
                    }
                }
                KeyCode::Down => {
                    if state.cursor + 1 < CHOICES.len() {
                        state.cursor += 1;
                    }
                }
                KeyCode::Char('1') => {
                    resolve_current(&mut state, Resolution::Ours);
                }
                KeyCode::Char('2') => {
                    resolve_current(&mut state, Resolution::Theirs);
                }
                KeyCode::Char('3') => {
                    resolve_current(&mut state, Resolution::Both);
                }
                KeyCode::Char('4') => {
                    resolve_current(&mut state, Resolution::Skip);
                }
                KeyCode::Char('a') => break ResolverResult::Aborted,
                KeyCode::Enter => {
                    let choice = CHOICES[state.cursor].1;
                    resolve_current(&mut state, choice);
                }
                _ => {}
            }

            // Check if all resolved
            if state.resolutions.iter().all(|r| r.is_some()) {
                let resolved: Vec<(String, Resolution)> = state
                    .conflicts
                    .iter()
                    .zip(state.resolutions.iter())
                    .filter_map(|(c, r)| r.map(|res| (c.path.clone(), res)))
                    .collect();
                break ResolverResult::Resolved(resolved);
            }
        }
    };

    super::restore_terminal()?;
    Ok(result)
}

fn resolve_current(state: &mut ResolverState, resolution: Resolution) {
    state.resolutions[state.current] = Some(resolution);
    // Advance to next unresolved
    for i in 0..state.conflicts.len() {
        let idx = (state.current + 1 + i) % state.conflicts.len();
        if state.resolutions[idx].is_none() {
            state.current = idx;
            state.cursor = 0;
            return;
        }
    }
}

fn draw_resolver(frame: &mut Frame, state: &ResolverState) {
    let area = frame.area();

    let header_area = Rect { height: 3, ..area };
    let conflict_area = Rect {
        y: 3,
        height: 4,
        ..area
    };
    let choices_area = Rect {
        y: 7,
        height: area.height.saturating_sub(10),
        ..area
    };
    let footer_area = Rect {
        y: area.height.saturating_sub(3),
        height: 3,
        ..area
    };

    let resolved_count = state.resolutions.iter().filter(|r| r.is_some()).count();
    let total = state.conflicts.len();

    let header = Paragraph::new(format!(
        " Resolving conflicts: {}/{} done",
        resolved_count, total,
    ))
    .block(Block::bordered().title(" Fuse Resolver "));
    frame.render_widget(header, header_area);

    let conflict = &state.conflicts[state.current];
    let status = match state.resolutions[state.current] {
        Some(Resolution::Ours) => " [→ OURS]",
        Some(Resolution::Theirs) => " [→ THEIRS]",
        Some(Resolution::Both) => " [→ BOTH]",
        Some(Resolution::Skip) => " [→ SKIPPED]",
        None => "",
    };

    let conflict_text = Paragraph::new(format!(
        " Conflict {} of {}: {}{}\n {}",
        state.current + 1,
        total,
        conflict.path,
        status,
        conflict.description,
    ))
    .block(Block::bordered());
    frame.render_widget(conflict_text, conflict_area);

    let items: Vec<ListItem> = CHOICES
        .iter()
        .enumerate()
        .map(|(i, (label, _))| {
            let marker = if i == state.cursor { "→" } else { " " };
            ListItem::new(format!("{} [{}] {}", marker, i + 1, label))
        })
        .collect();

    let list = List::new(items).block(Block::bordered().title(" Choose resolution "));
    frame.render_widget(list, choices_area);

    let footer = Paragraph::new(" ↑/↓ or 1-4 choose • Enter confirm • a abort • q quit")
        .block(Block::bordered());
    frame.render_widget(footer, footer_area);
}
