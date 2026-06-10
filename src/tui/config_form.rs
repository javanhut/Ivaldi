//! Standalone ratatui form for `ivaldi config` (no args / interactive mode).
//!
//! The first field is the scope — repo-local or global — and toggling it
//! reloads the form from (and saves to) the corresponding config file.
//! Sections: Scope, User, Appearance, Core, Remote (Remote only in local
//! scope). Text fields edit via the shared `TextInput` widget. Radio fields
//! toggle with left/right arrows.

use std::io;
use std::path::{Path, PathBuf};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::config::Config;
use crate::tui::input::TextInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    /// Local/global selector (radio; pseudo-field, never saved).
    Scope,
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
    /// Repo-local config path; `None` when not inside a repository.
    local_path: Option<PathBuf>,
    global_path: PathBuf,
    use_global: bool,
    fields: Vec<Field>,
    cursor: usize,
    editing: Option<TextInput>,
    dirty: bool,
    saved: bool,
    notice: Option<String>,
}

impl State {
    fn target_path(&self) -> &Path {
        if self.use_global {
            &self.global_path
        } else {
            // Safe: use_global is forced true when local_path is None.
            self.local_path.as_deref().unwrap()
        }
    }

    fn scope_label(&self) -> &'static str {
        if self.use_global { "global" } else { "local" }
    }

    /// (Re)build the field list from the currently selected config file.
    fn reload_fields(&mut self) {
        let cfg = Config::load(self.target_path()).unwrap_or_else(|_| Config::new());
        let mut fields = vec![
            Field {
                section: "Scope",
                key: "scope",
                label: "save to",
                kind: FieldKind::Scope,
                value: self.scope_label().to_string(),
            },
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
        // Per-repo concern; only meaningful in local scope.
        if !self.use_global {
            fields.push(Field {
                section: "Remote",
                key: "portal.default",
                label: "portal.default",
                kind: FieldKind::Text,
                value: cfg.get("portal.default").unwrap_or("").to_string(),
            });
        }
        self.fields = fields;
        self.cursor = self.cursor.min(self.fields.len() - 1);
        self.dirty = false;
    }

    /// Flip local ↔ global, reloading from the newly selected file.
    fn toggle_scope(&mut self) {
        if self.local_path.is_none() {
            self.notice = Some(
                "not inside an Ivaldi repository — only the global config is available".into(),
            );
            return;
        }
        let had_edits = self.dirty;
        self.use_global = !self.use_global;
        self.reload_fields();
        self.notice = Some(format!(
            "editing {} config: {}{}",
            self.scope_label(),
            self.target_path().display(),
            if had_edits {
                " (unsaved edits discarded)"
            } else {
                ""
            }
        ));
    }
}

/// Launch the interactive config form.
///
/// `local_path` is the repo's `.ivaldi/config` when inside a repository;
/// `start_global` selects the initial scope (forced when there is no repo).
pub fn run(local_path: Option<&Path>, global_path: &Path, start_global: bool) -> io::Result<()> {
    let mut state = State {
        use_global: start_global || local_path.is_none(),
        local_path: local_path.map(|p| p.to_path_buf()),
        global_path: global_path.to_path_buf(),
        fields: Vec::new(),
        cursor: 0,
        editing: None,
        dirty: false,
        saved: false,
        notice: None,
    };
    state.reload_fields();

    let mut terminal = super::init_terminal()?;
    let outcome = event_loop(&mut terminal, &mut state);
    super::restore_terminal()?;
    outcome?;

    if state.saved {
        let target = state.target_path().to_path_buf();
        // Rebuild a Config from state.fields and write it out.
        let mut out = Config::load(&target).unwrap_or_else(|_| Config::new());
        for f in &state.fields {
            if f.kind != FieldKind::Scope && !f.value.is_empty() {
                out.set(f.key, &f.value);
            }
        }
        out.save(&target)
            .map_err(|e| io::Error::other(e.to_string()))?;
        println!(
            "{} Configuration saved to {} ({})",
            crate::color::green("\u{2713}"),
            target.display(),
            state.scope_label()
        );
        if let Some(author) = out.author() {
            println!("Author: {}", crate::color::author(&author));
        }
    }
    Ok(())
}

fn event_loop<B: Backend>(terminal: &mut Terminal<B>, state: &mut State) -> io::Result<()> {
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
                    if field.key == "user.email" && !new_val.is_empty() && !is_email_like(&new_val)
                    {
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
                    state.notice =
                        Some("unsaved changes — press 's' to save or Esc to discard".into());
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
            KeyCode::Up | KeyCode::Char('k') if state.cursor > 0 => {
                state.cursor -= 1;
            }
            KeyCode::Down | KeyCode::Char('j') if state.cursor + 1 < state.fields.len() => {
                state.cursor += 1;
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Right | KeyCode::Char('l') => {
                match state.fields[state.cursor].kind {
                    FieldKind::Scope => state.toggle_scope(),
                    FieldKind::Bool => {
                        let field = &mut state.fields[state.cursor];
                        field.value = if field.value == "true" {
                            "false".into()
                        } else {
                            "true".into()
                        };
                        state.dirty = true;
                    }
                    FieldKind::Text => {}
                }
            }
            KeyCode::Enter => match state.fields[state.cursor].kind {
                FieldKind::Scope => state.toggle_scope(),
                FieldKind::Text => {
                    let field = &state.fields[state.cursor];
                    state.editing = Some(TextInput::with_value(field.value.clone()));
                }
                FieldKind::Bool => {
                    let f = &mut state.fields[state.cursor];
                    f.value = if f.value == "true" {
                        "false".into()
                    } else {
                        "true".into()
                    };
                    state.dirty = true;
                }
            },
            _ => {}
        }
    }
}

use crate::config::is_email_like;

fn draw(frame: &mut Frame, state: &State) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(3), // header
        Constraint::Min(5),    // form
        Constraint::Length(3), // footer
    ])
    .split(area);

    let header = Paragraph::new(format!(
        " Ivaldi Configuration ({})\n {}",
        state.scope_label(),
        state.target_path().display()
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
            FieldKind::Scope => {
                let global = state.use_global;
                let local_mark = if !global { "(●) local" } else { "( ) local" };
                let global_mark = if global { "(●) global" } else { "( ) global" };
                let style = if focused {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let mut spans = vec![
                    Span::raw(format!("   {} ", marker)),
                    Span::raw(format!("{:<16}", field.label)),
                    Span::styled(format!("{}  {}", local_mark, global_mark), style),
                ];
                if state.local_path.is_none() {
                    spans.push(Span::styled(
                        "  (no repo — global only)",
                        Style::default().add_modifier(Modifier::DIM),
                    ));
                }
                Line::from(spans)
            }
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
    if let Some(input) = state.editing.as_ref()
        && let Some(&(_, line_idx)) = field_rows.iter().find(|(i, _)| *i == state.cursor)
    {
        // chunks[1] has a 1-cell border on top.
        let y = chunks[1].y + 1 + line_idx as u16;
        // Field label is 16 wide + marker prefix (5) + "[" (1) = 22.
        let x = chunks[1].x + 1 + 5 + 16 + 1;
        let width = chunks[1]
            .width
            .saturating_sub(x - chunks[1].x)
            .saturating_sub(2);
        let input_area = Rect {
            x,
            y,
            width,
            height: 1,
        };
        // Clear the background rectangle and render the editor.
        frame.render_widget(Clear, input_area);
        input.render(
            frame,
            input_area,
            Style::default().add_modifier(Modifier::REVERSED),
        );
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
    let footer = Paragraph::new(format!("{}\n{}", hint, notice)).block(Block::bordered());
    frame.render_widget(footer, chunks[2]);
}
