//! Standalone ratatui form for `ivaldi config` (no args / interactive mode).
//!
//! Sections: User, Appearance, Core, Remote (only shown when inside a repo).
//! Text fields edit via the shared `TextInput` widget. Bool fields are radios
//! toggled with left/right arrows.

use std::io;
use std::path::Path;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::config::Config;
use crate::tui::input::TextInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Text,
    Bool,
}

struct Field {
    section: &'static str,
    key: &'static str,
    label: &'static str,
    kind: FieldKind,
    value: String,
}

struct State {
    path: String,
    inside_repo: bool,
    fields: Vec<Field>,
    cursor: usize,
    editing: Option<TextInput>,
    dirty: bool,
    saved: bool,
    notice: Option<String>,
}

/// Launch the interactive config form. Writes to `target_path` on save.
pub fn run(target_path: &Path, inside_repo: bool) -> io::Result<()> {
    let cfg = Config::load(target_path).unwrap_or_else(|_| Config::new());

    let mut fields = vec![
        Field {
            section: "User",
            key: "user.name",
            label: "name",
            kind: FieldKind::Text,
            value: cfg.get("user.name").unwrap_or("").to_string(),
        },
        Field {
            section: "User",
            key: "user.email",
            label: "email",
            kind: FieldKind::Text,
            value: cfg.get("user.email").unwrap_or("").to_string(),
        },
        Field {
            section: "Appearance",
            key: "color.ui",
            label: "color.ui",
            kind: FieldKind::Bool,
            value: cfg.get("color.ui").unwrap_or("true").to_string(),
        },
        Field {
            section: "Core",
            key: "core.autoshelf",
            label: "autoshelf",
            kind: FieldKind::Bool,
            value: cfg.get("core.autoshelf").unwrap_or("true").to_string(),
        },
    ];
    if inside_repo {
        fields.push(Field {
            section: "Remote",
            key: "portal.default",
            label: "portal.default",
            kind: FieldKind::Text,
            value: cfg.get("portal.default").unwrap_or("").to_string(),
        });
    }

    let mut state = State {
        path: target_path.display().to_string(),
        inside_repo,
        fields,
        cursor: 0,
        editing: None,
        dirty: false,
        saved: false,
        notice: None,
    };

    let mut terminal = super::init_terminal()?;
    let outcome = event_loop(&mut terminal, &mut state);
    super::restore_terminal()?;
    outcome?;

    if state.saved {
        // Rebuild a Config from state.fields and write it out.
        let mut out = Config::load(target_path).unwrap_or_else(|_| Config::new());
        for f in &state.fields {
            if !f.value.is_empty() {
                out.set(f.key, &f.value);
            }
        }
        out.save(target_path)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        println!(
            "{} Configuration saved to {}",
            crate::color::green("\u{2713}"),
            target_path.display()
        );
        if let Some(author) = out.author() {
            println!("Author: {}", crate::color::author(&author));
        }
    }
    Ok(())
}

fn event_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    state: &mut State,
) -> io::Result<()> {
    loop {
        terminal
            .draw(|frame| draw(frame, state))
            .map_err(|e| io::Error::other(e.to_string()))?;
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        // Edit mode: forward keys to the input, except Esc / Enter.
        if let Some(input) = state.editing.as_mut() {
            match key.code {
                KeyCode::Esc => {
                    state.editing = None;
                    state.notice = None;
                }
                KeyCode::Enter => {
                    let new_val = input.value.clone();
                    let field = &mut state.fields[state.cursor];
                    if field.key == "user.email" && !new_val.is_empty() && !is_email_like(&new_val) {
                        state.notice = Some(format!("'{}' doesn't look like an email", new_val));
                        // keep edit mode open so user can fix it
                    } else if field.key == "portal.default"
                        && !new_val.is_empty()
                        && crate::portal::parse_repo_spec(&new_val).is_err()
                    {
                        state.notice = Some(format!("'{}' is not a valid repo spec", new_val));
                    } else {
                        if field.value != new_val {
                            field.value = new_val;
                            state.dirty = true;
                        }
                        state.editing = None;
                        state.notice = None;
                    }
                }
                _ => {
                    input.handle_key(&key);
                }
            }
            continue;
        }

        match key.code {
            KeyCode::Char('q') => {
                if state.dirty {
                    state.notice = Some("unsaved changes — press 's' to save or Esc to discard".into());
                } else {
                    return Ok(());
                }
            }
            KeyCode::Esc => {
                // Esc without edit mode: discard and exit.
                state.saved = false;
                return Ok(());
            }
            KeyCode::Char('s') => {
                state.saved = true;
                return Ok(());
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if state.cursor > 0 {
                    state.cursor -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if state.cursor + 1 < state.fields.len() {
                    state.cursor += 1;
                }
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Right | KeyCode::Char('l') => {
                let field = &mut state.fields[state.cursor];
                if field.kind == FieldKind::Bool {
                    field.value = if field.value == "true" { "false".into() } else { "true".into() };
                    state.dirty = true;
                }
            }
            KeyCode::Enter => {
                let field = &state.fields[state.cursor];
                match field.kind {
                    FieldKind::Text => {
                        state.editing = Some(TextInput::with_value(field.value.clone()));
                    }
                    FieldKind::Bool => {
                        let f = &mut state.fields[state.cursor];
                        f.value = if f.value == "true" { "false".into() } else { "true".into() };
                        state.dirty = true;
                    }
                }
            }
            _ => {}
        }
    }
}

fn is_email_like(s: &str) -> bool {
    let (local, rest) = match s.split_once('@') {
        Some(p) => p,
        None => return false,
    };
    if local.is_empty() {
        return false;
    }
    rest.contains('.') && !rest.starts_with('.') && !rest.ends_with('.')
}

fn draw(frame: &mut Frame, state: &State) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(3), // header
        Constraint::Min(5),    // form
        Constraint::Length(3), // footer
    ])
    .split(area);

    let scope = if state.inside_repo { "repo-local" } else { "global" };
    let header = Paragraph::new(format!(
        " Ivaldi Configuration ({})\n {}",
        scope, state.path
    ))
    .block(Block::bordered().title(" Config "));
    frame.render_widget(header, chunks[0]);

    // Build form lines with section dividers.
    let mut lines: Vec<Line> = Vec::new();
    let mut last_section: &str = "";
    // Reserve one render-index per field (and section headers) so we know where
    // to draw the edit input.
    let mut field_rows: Vec<(usize, usize)> = Vec::new(); // (field_idx, line_idx)

    for (i, field) in state.fields.iter().enumerate() {
        if field.section != last_section {
            if !lines.is_empty() {
                lines.push(Line::raw(""));
            }
            lines.push(Line::styled(
                format!(" {}", field.section),
                Style::default().add_modifier(Modifier::BOLD),
            ));
            last_section = field.section;
        }

        let focused = i == state.cursor;
        let marker = if focused { "▸" } else { " " };

        let row = match field.kind {
            FieldKind::Text => {
                let shown = if field.value.is_empty() {
                    "(empty)".to_string()
                } else {
                    field.value.clone()
                };
                let style = if focused {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else if field.value.is_empty() {
                    Style::default().add_modifier(Modifier::DIM)
                } else {
                    Style::default()
                };
                Line::from(vec![
                    Span::raw(format!("   {} ", marker)),
                    Span::raw(format!("{:<16}", field.label)),
                    Span::styled(format!("[{}]", shown), style),
                ])
            }
            FieldKind::Bool => {
                let on = field.value == "true";
                let on_mark = if on { "(●) true" } else { "( ) true" };
                let off_mark = if !on { "(●) false" } else { "( ) false" };
                let style = if focused {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Line::from(vec![
                    Span::raw(format!("   {} ", marker)),
                    Span::raw(format!("{:<16}", field.label)),
                    Span::styled(format!("{}  {}", on_mark, off_mark), style),
                ])
            }
        };
        field_rows.push((i, lines.len()));
        lines.push(row);
    }

    let form = Paragraph::new(lines).block(Block::bordered());
    frame.render_widget(form, chunks[1]);

    // If editing a text field, draw the TextInput over the field's line.
    if let Some(input) = state.editing.as_ref() {
        if let Some(&(_, line_idx)) = field_rows.iter().find(|(i, _)| *i == state.cursor) {
            // chunks[1] has a 1-cell border on top.
            let y = chunks[1].y + 1 + line_idx as u16;
            // Field label is 16 wide + marker prefix (5) + "[" (1) = 22.
            let x = chunks[1].x + 1 + 5 + 16 + 1;
            let width = chunks[1].width.saturating_sub(x - chunks[1].x).saturating_sub(2);
            let input_area = Rect {
                x,
                y,
                width,
                height: 1,
            };
            // Clear the background rectangle and render the editor.
            frame.render_widget(Clear, input_area);
            input.render(frame, input_area, Style::default().add_modifier(Modifier::REVERSED));
        }
    }

    let hint = if state.editing.is_some() {
        " [Enter] Confirm  [Esc] Cancel"
    } else {
        " [↑↓] Navigate  [Enter] Edit  [←→] Toggle  [s] Save  [q] Quit"
    };
    let notice = state
        .notice
        .as_deref()
        .unwrap_or(if state.dirty { " (modified)" } else { "" });
    let footer = Paragraph::new(format!("{}\n{}", hint, notice))
        .block(Block::bordered());
    frame.render_widget(footer, chunks[2]);
}
