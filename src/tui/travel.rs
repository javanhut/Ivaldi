//! Interactive time travel — browse commit history with arrow keys.
//!
//! Features:
//! - Fixed-window scrollable list
//! - Visual scroll indicator
//! - Arrow key navigation
//! - Enter to select → Diverge (new timeline) or Overwrite (reset)
//! - Search filtering
//! - Home/End/1-9 jump

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::repo::HistoryEntry;

/// Action chosen by the user after selecting a seal.
#[derive(Debug, Clone)]
pub enum TravelAction {
    /// Create a new timeline from this seal.
    Diverge { seal_index: u64, new_timeline: String },
    /// Reset current timeline to this seal.
    Overwrite { seal_index: u64 },
    /// User cancelled.
    Cancel,
}

struct TravelState {
    entries: Vec<HistoryEntry>,
    cursor: usize,
    offset: usize,
    search: Option<String>,
    timeline: String,
}

impl TravelState {
    fn filtered_entries(&self) -> Vec<&HistoryEntry> {
        match &self.search {
            Some(q) => {
                let q = q.to_lowercase();
                self.entries.iter().filter(|e| {
                    e.message.to_lowercase().contains(&q)
                        || e.author.to_lowercase().contains(&q)
                        || e.seal_name.to_lowercase().contains(&q)
                }).collect()
            }
            None => self.entries.iter().collect(),
        }
    }

    fn total(&self) -> usize {
        self.filtered_entries().len()
    }
}

/// Run the interactive travel TUI. Returns the user's action.
pub fn run_travel(
    entries: Vec<HistoryEntry>,
    timeline: &str,
    search: Option<String>,
) -> std::io::Result<TravelAction> {
    if entries.is_empty() {
        eprintln!("No commits to travel through.");
        return Ok(TravelAction::Cancel);
    }

    let mut terminal = super::init_terminal()?;
    let mut state = TravelState {
        entries,
        cursor: 0,
        offset: 0,
        search,
        timeline: timeline.to_string(),
    };

    let result = loop {
        terminal.draw(|frame| draw_travel(frame, &state))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press { continue; }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break TravelAction::Cancel,
                KeyCode::Up => {
                    if state.cursor > 0 { state.cursor -= 1; }
                    adjust_offset(&mut state, frame_height());
                }
                KeyCode::Down => {
                    if state.cursor + 1 < state.total() { state.cursor += 1; }
                    adjust_offset(&mut state, frame_height());
                }
                KeyCode::Home => { state.cursor = 0; state.offset = 0; }
                KeyCode::End => {
                    let total = state.total();
                    if total > 0 { state.cursor = total - 1; }
                    adjust_offset(&mut state, frame_height());
                }
                KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                    let n = (c as usize) - ('0' as usize);
                    if n <= state.total() { state.cursor = n - 1; }
                    adjust_offset(&mut state, frame_height());
                }
                KeyCode::Enter => {
                    let filtered = state.filtered_entries();
                    if let Some(entry) = filtered.get(state.cursor) {
                        let idx = entry.index;
                        super::restore_terminal()?;
                        return prompt_travel_action(idx);
                    }
                }
                _ => {}
            }
        }
    };

    super::restore_terminal()?;
    Ok(result)
}

fn frame_height() -> usize { 10 } // approximate visible items

fn adjust_offset(state: &mut TravelState, visible: usize) {
    if state.cursor < state.offset {
        state.offset = state.cursor;
    } else if state.cursor >= state.offset + visible {
        state.offset = state.cursor - visible + 1;
    }
}

fn draw_travel(frame: &mut Frame, state: &TravelState) {
    let area = frame.area();
    let filtered = state.filtered_entries();
    let total = filtered.len();

    // Header
    let header_area = Rect { height: 3, ..area };
    let list_area = Rect { y: 3, height: area.height.saturating_sub(6), ..area };
    let footer_area = Rect { y: area.height.saturating_sub(3), height: 3, ..area };

    let visible = list_area.height as usize;

    // Header block
    let header = Paragraph::new(format!(
        " ⏱ Seals in timeline '{}'\n Showing {}-{} of {}",
        state.timeline,
        state.offset + 1,
        (state.offset + visible).min(total),
        total,
    ))
    .block(Block::bordered().title(" Travel "));
    frame.render_widget(header, header_area);

    // Entry list
    let items: Vec<ListItem> = filtered.iter().enumerate()
        .skip(state.offset)
        .take(visible)
        .map(|(i, entry)| {
            let marker = if i == state.cursor { "→" } else { " " };
            let head = if i == 0 { " [HEAD]" } else { "" };
            let text = format!(
                "{} {}. {} ({}){}\n     {}\n     {} • {}",
                marker, i + 1, entry.seal_name, entry.short_hash, head,
                entry.message,
                entry.author, entry.time_unix,
            );
            ListItem::new(text)
        })
        .collect();

    let list = List::new(items)
        .block(Block::bordered())
        .highlight_style(Style::default().bold());
    frame.render_widget(list, list_area);

    // Footer
    let footer = Paragraph::new(" ↑/↓ navigate • Enter select • Home/End jump • 1-9 goto • q quit")
        .block(Block::bordered());
    frame.render_widget(footer, footer_area);
}

fn prompt_travel_action(seal_index: u64) -> std::io::Result<TravelAction> {
    println!("\nSelected seal at index {}", seal_index);
    println!("\n? What would you like to do?");
    println!("  1. Diverge - Create new timeline from this seal");
    println!("  2. Overwrite - Reset current timeline");
    println!("  3. Cancel");
    print!("\nChoice: ");
    use std::io::Write;
    std::io::stdout().flush()?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    match input.trim() {
        "1" => {
            print!("Enter new timeline name: ");
            std::io::stdout().flush()?;
            let mut name = String::new();
            std::io::stdin().read_line(&mut name)?;
            let name = name.trim().to_string();
            if name.is_empty() {
                Ok(TravelAction::Cancel)
            } else {
                Ok(TravelAction::Diverge { seal_index, new_timeline: name })
            }
        }
        "2" => {
            print!("WARNING: This will remove commits. Type 'yes' to confirm: ");
            std::io::stdout().flush()?;
            let mut confirm = String::new();
            std::io::stdin().read_line(&mut confirm)?;
            if confirm.trim() == "yes" {
                Ok(TravelAction::Overwrite { seal_index })
            } else {
                println!("Aborted.");
                Ok(TravelAction::Cancel)
            }
        }
        _ => Ok(TravelAction::Cancel),
    }
}
