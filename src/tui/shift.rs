//! Interactive commit squash — select range with arrow keys.
//!
//! Two-phase selection:
//! 1. Select START commit (oldest)
//! 2. Select END commit (newest)
//!
//! Then review, enter message, confirm.

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::repo::HistoryEntry;

/// Result of the interactive shift selection.
#[derive(Debug)]
pub enum ShiftAction {
    /// User selected a range to squash.
    Squash {
        start_index: u64,
        end_index: u64,
        message: String,
    },
    /// User cancelled.
    Cancel,
}

struct ShiftState {
    entries: Vec<HistoryEntry>,
    cursor: usize,
    phase: Phase,
    start: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    SelectStart,
    SelectEnd,
}

/// Run the interactive shift TUI.
pub fn run_shift(entries: Vec<HistoryEntry>) -> std::io::Result<ShiftAction> {
    if entries.len() < 2 {
        eprintln!("Need at least 2 commits to squash.");
        return Ok(ShiftAction::Cancel);
    }

    let mut terminal = super::init_terminal()?;
    let mut state = ShiftState {
        entries,
        cursor: 0,
        phase: Phase::SelectStart,
        start: None,
    };

    let result = loop {
        terminal.draw(|frame| draw_shift(frame, &state))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break ShiftAction::Cancel,
                KeyCode::Up if state.cursor > 0 => {
                    state.cursor -= 1;
                }
                KeyCode::Down if state.cursor + 1 < state.entries.len() => {
                    state.cursor += 1;
                }
                KeyCode::Enter => {
                    match state.phase {
                        Phase::SelectStart => {
                            state.start = Some(state.cursor);
                            state.phase = Phase::SelectEnd;
                            // Move cursor to top for end selection
                            state.cursor = 0;
                        }
                        Phase::SelectEnd => {
                            let start_pos = state.start.unwrap();
                            let end_pos = state.cursor;

                            // Entries are newest-first, so end_pos <= start_pos
                            // for squashing we need oldest..newest
                            let (oldest, newest) = if start_pos > end_pos {
                                (start_pos, end_pos)
                            } else {
                                (end_pos, start_pos)
                            };

                            let start_idx = state.entries[oldest].index;
                            let end_idx = state.entries[newest].index;
                            let count = oldest - newest + 1;

                            super::restore_terminal()?;

                            // Show review and prompt for message
                            println!("\n✓ Range selected: {} commits will be squashed\n", count);
                            println!("📋 Review commits to squash:\n");
                            for i in (newest..=oldest).rev() {
                                println!(
                                    "[✓] {} - {}",
                                    state.entries[i].short_hash, state.entries[i].message
                                );
                            }

                            // Generate suggested message
                            let suggested: Vec<&str> = (newest..=oldest)
                                .rev()
                                .map(|i| state.entries[i].message.as_str())
                                .collect();
                            let suggested_msg =
                                format!("Squashed {} commits:\n\n{}", count, suggested.join("\n"));

                            println!("\nSuggested commit message:\n{}\n", suggested_msg);
                            print!("Enter new commit message (or press Enter to use suggested): ");
                            use std::io::Write;
                            std::io::stdout().flush()?;

                            let mut input = String::new();
                            std::io::stdin().read_line(&mut input)?;
                            let message = if input.trim().is_empty() {
                                suggested_msg
                            } else {
                                input.trim().to_string()
                            };

                            print!(
                                "\n⚠ WARNING: This will rewrite commit history!\nConfirm squash? (yes/no): "
                            );
                            std::io::stdout().flush()?;
                            let mut confirm = String::new();
                            std::io::stdin().read_line(&mut confirm)?;

                            if confirm.trim() == "yes" {
                                return Ok(ShiftAction::Squash {
                                    start_index: start_idx,
                                    end_index: end_idx,
                                    message,
                                });
                            } else {
                                println!("Aborted.");
                                return Ok(ShiftAction::Cancel);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    };

    super::restore_terminal()?;
    Ok(result)
}

fn draw_shift(frame: &mut Frame, state: &ShiftState) {
    let area = frame.area();

    let header_area = Rect { height: 3, ..area };
    let list_area = Rect {
        y: 3,
        height: area.height.saturating_sub(6),
        ..area
    };
    let footer_area = Rect {
        y: area.height.saturating_sub(3),
        height: 3,
        ..area
    };

    let phase_text = match state.phase {
        Phase::SelectStart => "⏱ Select START of commit range (oldest)",
        Phase::SelectEnd => "⏱ Select END of commit range (newest)",
    };

    let header = Paragraph::new(phase_text).block(Block::bordered().title(" Shift "));
    frame.render_widget(header, header_area);

    let visible = list_area.height as usize;
    let offset = if state.cursor >= visible {
        state.cursor - visible + 1
    } else {
        0
    };

    let items: Vec<ListItem> = state
        .entries
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible)
        .map(|(i, entry)| {
            let marker = if i == state.cursor { "→" } else { " " };
            let start_mark = if state.start == Some(i) {
                " [START]"
            } else {
                ""
            };
            let text = format!(
                "{} {}. {} ({}){}\n     {}",
                marker,
                i + 1,
                entry.seal_name,
                entry.short_hash,
                start_mark,
                entry.message,
            );
            ListItem::new(text)
        })
        .collect();

    let list = List::new(items).block(Block::bordered());
    frame.render_widget(list, list_area);

    let footer = Paragraph::new(" ↑/↓ navigate • Enter select • q quit").block(Block::bordered());
    frame.render_widget(footer, footer_area);
}
