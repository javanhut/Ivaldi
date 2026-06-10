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
    Diverge {
        seal_index: u64,
        new_timeline: String,
    },
    /// Reset current timeline to this seal.
    Overwrite { seal_index: u64 },
    /// User cancelled.
    Cancel,
}

/// Lines each entry occupies in the list view (1 header + 1 message + 1
/// author/time). Kept as a constant so the scrolling math stays in sync
/// with the rendering.
const ENTRY_LINES: usize = 3;
/// Blank row between entries for readability.
const ENTRY_SPACING: usize = 1;
/// Total rows per entry slot in the list area.
const ENTRY_SLOT: usize = ENTRY_LINES + ENTRY_SPACING;

struct TravelState {
    entries: Vec<HistoryEntry>,
    cursor: usize,
    offset: usize,
    search: Option<String>,
    timeline: String,
    /// Real visible-entry capacity, recomputed each frame from the
    /// terminal height. Cached so non-render code paths (Down handler,
    /// PgDn) can keep the scroll offset consistent with what was last
    /// drawn.
    viewport: usize,
}

impl TravelState {
    fn filtered_entries(&self) -> Vec<&HistoryEntry> {
        match &self.search {
            Some(q) => {
                let q = q.to_lowercase();
                self.entries
                    .iter()
                    .filter(|e| {
                        e.message.to_lowercase().contains(&q)
                            || e.author.to_lowercase().contains(&q)
                            || e.seal_name.to_lowercase().contains(&q)
                    })
                    .collect()
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
        viewport: 1,
    };

    let result = loop {
        terminal.draw(|frame| draw_travel(frame, &mut state))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let viewport = state.viewport.max(1);
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break TravelAction::Cancel,
                KeyCode::Up => {
                    if state.cursor > 0 {
                        state.cursor -= 1;
                    }
                    adjust_offset(&mut state, viewport);
                }
                KeyCode::Down => {
                    if state.cursor + 1 < state.total() {
                        state.cursor += 1;
                    }
                    adjust_offset(&mut state, viewport);
                }
                KeyCode::PageUp => {
                    state.cursor = state.cursor.saturating_sub(viewport);
                    adjust_offset(&mut state, viewport);
                }
                KeyCode::PageDown => {
                    let total = state.total();
                    if total > 0 {
                        state.cursor = (state.cursor + viewport).min(total - 1);
                    }
                    adjust_offset(&mut state, viewport);
                }
                KeyCode::Home => {
                    state.cursor = 0;
                    state.offset = 0;
                }
                KeyCode::End => {
                    let total = state.total();
                    if total > 0 {
                        state.cursor = total - 1;
                    }
                    adjust_offset(&mut state, viewport);
                }
                KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                    let n = (c as usize) - ('0' as usize);
                    if n <= state.total() {
                        state.cursor = n - 1;
                    }
                    adjust_offset(&mut state, viewport);
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

/// Keep the visible window aligned with the cursor. `visible` is the
/// number of entries (not rows) the list area can hold this frame.
fn adjust_offset(state: &mut TravelState, visible: usize) {
    let visible = visible.max(1);
    if state.cursor < state.offset {
        state.offset = state.cursor;
    } else if state.cursor >= state.offset + visible {
        state.offset = state.cursor + 1 - visible;
    }
}

fn draw_travel(frame: &mut Frame, state: &mut TravelState) {
    let area = frame.area();

    // Header (3 rows) + footer (3 rows) bracket the list area. Use
    // saturating math so a 5-row terminal still renders without panic.
    let header_height: u16 = 3;
    let footer_height: u16 = 3;
    let header_area = Rect {
        height: header_height,
        ..area
    };
    let list_area = Rect {
        y: area.y + header_height,
        height: area.height.saturating_sub(header_height + footer_height),
        ..area
    };
    let footer_area = Rect {
        y: area.height.saturating_sub(footer_height),
        height: footer_height,
        ..area
    };

    // Inside the bordered list block we lose 2 rows (top + bottom border).
    let inner_rows = list_area.height.saturating_sub(2) as usize;
    // Each entry is `ENTRY_LINES` lines + `ENTRY_SPACING` blank rows
    // separator. The very last entry doesn't need the trailing blank,
    // so capacity is computed by allowing the last slot to shed its
    // spacer.
    let viewport = if inner_rows >= ENTRY_LINES {
        ((inner_rows + ENTRY_SPACING) / ENTRY_SLOT).max(1)
    } else {
        1
    };
    state.viewport = viewport;

    // Compute the filtered view AFTER mutating state, so we don't fight
    // the borrow checker.
    let filtered = state.filtered_entries();
    let total = filtered.len();

    // Re-clamp the offset in case the terminal shrunk between frames
    // (e.g., user resized the window). `adjust_offset` uses the cached
    // viewport, but a resize can happen without a key press.
    let mut offset = state.offset;
    if state.cursor < offset {
        offset = state.cursor;
    } else if state.cursor >= offset + viewport {
        offset = state.cursor + 1 - viewport;
    }
    // Note: we recompute on the local `offset` and write back below
    // (after the immutable borrow from `filtered_entries()` is done).

    let last_visible = (offset + viewport).min(total);

    // ---- Header
    let header = Paragraph::new(format!(
        " ⏱ Seals in timeline '{}'  ·  showing {}-{} of {}",
        state.timeline,
        if total == 0 { 0 } else { offset + 1 },
        last_visible,
        total,
    ))
    .block(Block::bordered().title(" Travel "));
    frame.render_widget(header, header_area);

    // ---- List block (background + border)
    frame.render_widget(Block::bordered(), list_area);

    // ---- Per-entry render with deterministic geometry
    //
    // Manual layout instead of ratatui's `List` widget. Multi-line
    // `ListItem`s in 0.30 don't always lay out the way we want, so we
    // compute each entry's row range explicitly and render with
    // `Paragraph`. Each entry occupies `ENTRY_LINES` rows + an
    // `ENTRY_SPACING`-row gutter, except the last visible one (no
    // trailing gutter).
    let inner_x = list_area.x + 1;
    let inner_y = list_area.y + 1;
    let inner_width = list_area.width.saturating_sub(2);
    for (slot, (i, entry)) in filtered
        .iter()
        .enumerate()
        .skip(offset)
        .take(viewport)
        .enumerate()
    {
        let row = inner_y + (slot * ENTRY_SLOT) as u16;
        // Don't draw past the inner area — be defensive against tiny
        // terminals or rounding.
        if row + ENTRY_LINES as u16 > inner_y + inner_rows as u16 {
            break;
        }
        let entry_area = Rect {
            x: inner_x,
            y: row,
            width: inner_width,
            height: ENTRY_LINES as u16,
        };

        let is_cursor = i == state.cursor;
        let marker = if is_cursor { "→" } else { " " };
        let head_tag = if i == 0 { " [HEAD]" } else { "" };
        let style = if is_cursor {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let lines = vec![
            Line::from(format!(
                "{} {}. {} ({}){}",
                marker,
                i + 1,
                entry.seal_name,
                entry.short_hash,
                head_tag,
            ))
            .style(style),
            Line::from(format!("     {}", first_line(&entry.message))).style(style),
            Line::from(format!("     {} · {}", entry.author, entry.time_unix)).style(style),
        ];
        let para = Paragraph::new(lines);
        frame.render_widget(para, entry_area);
    }

    // ---- Footer
    let footer = Paragraph::new(
        " ↑/↓ navigate · PgUp/PgDn page · Home/End jump · 1-9 goto · Enter select · q quit",
    )
    .block(Block::bordered());
    frame.render_widget(footer, footer_area);

    // Persist the clamped offset so the next key press uses the same
    // viewport math the renderer just used.
    drop(filtered);
    state.offset = offset;
}

/// First line of a (possibly multi-line) commit message, trimmed.
fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim_end()
}

/// Compute how many entries fit in `inner_rows` of vertical space, given
/// each entry slot is `ENTRY_LINES` content + `ENTRY_SPACING` gutter.
/// Pulled out as a free function so the math is testable without a Frame.
#[cfg(test)]
fn viewport_entries(inner_rows: usize) -> usize {
    if inner_rows >= ENTRY_LINES {
        ((inner_rows + ENTRY_SPACING) / ENTRY_SLOT).max(1)
    } else {
        1
    }
}

fn prompt_travel_action(seal_index: u64) -> std::io::Result<TravelAction> {
    println!("\nSelected seal at index {}", seal_index);
    println!("\n? What would you like to do?");
    println!("  1. Diverge - Create new timeline from this seal");
    println!("  2. Overwrite - Move current timeline back to this seal");
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
                Ok(TravelAction::Diverge {
                    seal_index,
                    new_timeline: name,
                })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(n: usize) -> Vec<HistoryEntry> {
        use crate::hash::B3Hash;
        (0..n)
            .map(|i| {
                let hash = B3Hash::digest(&[i as u8]);
                HistoryEntry {
                    index: i as u64,
                    hash,
                    seal_name: format!("seal-{}", i),
                    short_hash: hash.short8(),
                    author: "tester".into(),
                    message: format!("msg {}", i),
                    time_unix: i as i64,
                    timeline: "main".into(),
                    is_merge: false,
                }
            })
            .collect()
    }

    fn state(n: usize) -> TravelState {
        TravelState {
            entries: entries(n),
            cursor: 0,
            offset: 0,
            search: None,
            timeline: "main".into(),
            viewport: 5,
        }
    }

    #[test]
    fn viewport_entries_for_typical_terminal() {
        // Inner area = 30 rows → entry slot = 4 rows → 7 entries fit
        // (4*7 = 28; the 8th would need 28+4 > 30).
        assert_eq!(viewport_entries(30), 7);
        // Inner area = 4 rows → exactly one entry fits (3 content + spacer).
        assert_eq!(viewport_entries(4), 1);
        // Inner area = 0 → still 1 (we never claim 0 capacity).
        assert_eq!(viewport_entries(0), 1);
    }

    #[test]
    fn adjust_offset_does_not_skip_entries_when_viewport_matches_real_height() {
        // 342 entries, viewport of 7 (typical terminal). The previous
        // code hard-coded 10 here regardless of real viewport, which
        // caused the "10 down arrows then a blank" symptom.
        let mut s = state(342);
        s.viewport = 7;
        // Step the cursor through the first viewport — offset stays 0.
        for _ in 0..6 {
            s.cursor += 1;
            let v = s.viewport;
            adjust_offset(&mut s, v);
            assert_eq!(s.offset, 0, "cursor still in viewport");
        }
        // 7th press scrolls by 1 (cursor=7, offset becomes 1).
        s.cursor += 1;
        let v = s.viewport;
        adjust_offset(&mut s, v);
        assert_eq!(s.cursor, 7);
        assert_eq!(s.offset, 1);

        // Cursor should always be within [offset, offset+viewport).
        assert!(s.cursor >= s.offset);
        assert!(s.cursor < s.offset + s.viewport);
    }

    #[test]
    fn adjust_offset_with_tiny_viewport_still_advances() {
        // Worst case: 1-entry viewport. Each press moves the offset by 1.
        let mut s = state(10);
        s.viewport = 1;
        for expected in 1..10 {
            s.cursor += 1;
            let v = s.viewport;
            adjust_offset(&mut s, v);
            assert_eq!(s.cursor, expected);
            assert_eq!(s.offset, expected, "viewport=1 → offset tracks cursor");
        }
    }

    #[test]
    fn adjust_offset_clamps_when_cursor_jumps_back() {
        // Home key (cursor → 0) should snap offset to 0 too.
        let mut s = state(50);
        s.viewport = 7;
        s.cursor = 30;
        s.offset = 24;
        s.cursor = 0;
        let v = s.viewport;
        adjust_offset(&mut s, v);
        assert_eq!(s.offset, 0);
    }
}
