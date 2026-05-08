//! Pre-dashboard launcher.
//!
//! Shown when `ivaldi tui` is invoked outside any repository. Lets the
//! user pick between cloning a remote repo, initialising a new one, or
//! opening an existing one — then returns the choice so the caller can
//! enter the dashboard pointed at the resulting work tree.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::tui::input::TextInput;
use crate::tui::theme::Theme;

/// The user's choice from the launcher screen. The caller is responsible
/// for actually performing the operation; the launcher only gathers input.
pub enum LauncherChoice {
    /// Clone a remote repo. `target_dir` is where to clone into (relative
    /// or absolute path); `repo_arg` is owner/repo, full URL, or shorthand
    /// — whatever `parse_repo_arg` accepts.
    Download {
        repo_arg: String,
        target_dir: PathBuf,
    },
    /// Initialise a new Ivaldi repository at the given directory.
    Forge { target_dir: PathBuf },
    /// Open an existing repository. The launcher does not validate the
    /// path; the caller decides whether to surface a "not a repo" error.
    Open { target_dir: PathBuf },
    /// User pressed q / Ctrl+C / Esc on the main menu.
    Quit,
}

/// The four top-level choices on the main menu.
#[derive(Clone, Copy)]
enum MenuItem {
    Download,
    Forge,
    Open,
    Quit,
}

impl MenuItem {
    const ALL: [MenuItem; 4] = [
        MenuItem::Download,
        MenuItem::Forge,
        MenuItem::Open,
        MenuItem::Quit,
    ];

    fn label(self) -> &'static str {
        match self {
            MenuItem::Download => "Download — clone a remote repository (GitHub or GitLab)",
            MenuItem::Forge => "Forge    — initialise a new repository in a directory",
            MenuItem::Open => "Open     — open an existing repository",
            MenuItem::Quit => "Quit",
        }
    }
}

/// Which screen the launcher is currently displaying.
enum Stage {
    /// Top-level four-item menu.
    Menu { cursor: usize },
    /// Two-field form: repo argument + target directory. Submitted = Download.
    DownloadForm {
        repo_input: TextInput,
        dir_input: TextInput,
        focus: FormFocus,
        error: Option<String>,
    },
    /// Single-field form for forge/open.
    PathForm {
        title: &'static str,
        purpose: PathFormPurpose,
        input: TextInput,
        error: Option<String>,
    },
}

#[derive(Clone, Copy)]
enum PathFormPurpose {
    Forge,
    Open,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FormFocus {
    Repo,
    Dir,
}

impl FormFocus {
    fn toggle(self) -> Self {
        match self {
            FormFocus::Repo => FormFocus::Dir,
            FormFocus::Dir => FormFocus::Repo,
        }
    }
}

/// Run the launcher. Owns terminal init/teardown — same pattern as
/// `tui::travel::run_travel`.
pub fn run() -> io::Result<LauncherChoice> {
    let mut terminal = crate::tui::init_terminal()?;
    let theme = Theme::default_theme();
    let mut stage = Stage::Menu { cursor: 0 };

    let result = (|| -> io::Result<LauncherChoice> {
        loop {
            terminal.draw(|frame| render(frame, &stage, &theme))?;

            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match handle_key(&mut stage, &key) {
                Transition::Stay => {}
                Transition::Done(choice) => return Ok(choice),
            }
        }
    })();

    let _ = crate::tui::restore_terminal();
    result
}

enum Transition {
    Stay,
    Done(LauncherChoice),
}

fn handle_key(stage: &mut Stage, key: &KeyEvent) -> Transition {
    match stage {
        Stage::Menu { cursor } => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Transition::Done(LauncherChoice::Quit),
            KeyCode::Char('j') | KeyCode::Down => {
                if *cursor + 1 < MenuItem::ALL.len() {
                    *cursor += 1;
                }
                Transition::Stay
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if *cursor > 0 {
                    *cursor -= 1;
                }
                Transition::Stay
            }
            KeyCode::Enter => match MenuItem::ALL[*cursor] {
                MenuItem::Download => {
                    *stage = Stage::DownloadForm {
                        repo_input: TextInput::new(),
                        dir_input: TextInput::new(),
                        focus: FormFocus::Repo,
                        error: None,
                    };
                    Transition::Stay
                }
                MenuItem::Forge => {
                    *stage = Stage::PathForm {
                        title: "Forge new repository",
                        purpose: PathFormPurpose::Forge,
                        input: TextInput::with_value(default_path()),
                        error: None,
                    };
                    Transition::Stay
                }
                MenuItem::Open => {
                    *stage = Stage::PathForm {
                        title: "Open existing repository",
                        purpose: PathFormPurpose::Open,
                        input: TextInput::with_value(default_path()),
                        error: None,
                    };
                    Transition::Stay
                }
                MenuItem::Quit => Transition::Done(LauncherChoice::Quit),
            },
            _ => Transition::Stay,
        },
        Stage::DownloadForm {
            repo_input,
            dir_input,
            focus,
            error,
        } => match key.code {
            KeyCode::Esc => {
                *stage = Stage::Menu { cursor: 0 };
                Transition::Stay
            }
            KeyCode::Tab => {
                *focus = focus.toggle();
                Transition::Stay
            }
            KeyCode::Enter => {
                let repo = repo_input.value.trim().to_string();
                if repo.is_empty() {
                    *error = Some("Repository required (e.g. owner/repo)".into());
                    return Transition::Stay;
                }
                let dir = dir_input.value.trim();
                let target_dir = if dir.is_empty() {
                    derive_target_dir(&repo)
                } else {
                    PathBuf::from(dir)
                };
                Transition::Done(LauncherChoice::Download {
                    repo_arg: repo,
                    target_dir,
                })
            }
            _ => {
                let active = match focus {
                    FormFocus::Repo => repo_input,
                    FormFocus::Dir => dir_input,
                };
                active.handle_key(key);
                Transition::Stay
            }
        },
        Stage::PathForm {
            purpose,
            input,
            error,
            ..
        } => match key.code {
            KeyCode::Esc => {
                *stage = Stage::Menu { cursor: 0 };
                Transition::Stay
            }
            KeyCode::Enter => {
                let raw = input.value.trim();
                if raw.is_empty() {
                    *error = Some("Path required".into());
                    return Transition::Stay;
                }
                let path = PathBuf::from(raw);
                let choice = match purpose {
                    PathFormPurpose::Forge => LauncherChoice::Forge { target_dir: path },
                    PathFormPurpose::Open => LauncherChoice::Open { target_dir: path },
                };
                Transition::Done(choice)
            }
            _ => {
                input.handle_key(key);
                Transition::Stay
            }
        },
    }
}

/// Pull the repo name out of `owner/repo`, `host:owner/repo.git`, etc., to
/// suggest a default target directory. Falls back to "ivaldi-clone".
fn derive_target_dir(repo_arg: &str) -> PathBuf {
    let trimmed = repo_arg.trim_end_matches(".git");
    let last = trimmed
        .rsplit('/')
        .next()
        .or_else(|| trimmed.rsplit(':').next())
        .unwrap_or("");
    if last.is_empty() {
        PathBuf::from("ivaldi-clone")
    } else {
        PathBuf::from(last)
    }
}

fn default_path() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string())
}

fn render(frame: &mut Frame, stage: &Stage, theme: &Theme) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let title = Paragraph::new(Line::from(vec![
        Span::styled("Ivaldi", theme.brand),
        Span::raw("  "),
        Span::styled("— no repository here", theme.dim),
    ]));
    let title_area = Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4),
        height: 1,
    };
    frame.render_widget(title, title_area);

    let body_area = Rect {
        x: area.x + 4,
        y: area.y + 4,
        width: area.width.saturating_sub(8),
        height: area.height.saturating_sub(6),
    };

    match stage {
        Stage::Menu { cursor } => render_menu(frame, body_area, *cursor, theme),
        Stage::DownloadForm {
            repo_input,
            dir_input,
            focus,
            error,
        } => render_download_form(frame, body_area, repo_input, dir_input, *focus, error, theme),
        Stage::PathForm {
            title,
            input,
            error,
            ..
        } => render_path_form(frame, body_area, title, input, error, theme),
    }

    // Footer
    let footer_area = Rect {
        x: area.x + 2,
        y: area.y + area.height.saturating_sub(2),
        width: area.width.saturating_sub(4),
        height: 1,
    };
    let footer = match stage {
        Stage::Menu { .. } => " j/k:move  Enter:select  q/Esc:quit",
        Stage::DownloadForm { .. } => " Tab:switch field  Enter:download  Esc:back",
        Stage::PathForm { .. } => " Enter:confirm  Esc:back",
    };
    frame.render_widget(Paragraph::new(Span::styled(footer, theme.dim)), footer_area);
}

fn render_menu(frame: &mut Frame, area: Rect, cursor: usize, theme: &Theme) {
    let items: Vec<ListItem> = MenuItem::ALL
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let marker = if i == cursor { ">" } else { " " };
            let line = format!("{} {}", marker, item.label());
            let style = if i == cursor {
                theme.cursor
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Span::styled(line, style))
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.brand)
        .title(Span::styled(" Start ", theme.title));
    frame.render_widget(List::new(items).block(block), area);
}

fn render_download_form(
    frame: &mut Frame,
    area: Rect,
    repo_input: &TextInput,
    dir_input: &TextInput,
    focus: FormFocus,
    error: &Option<String>,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.brand)
        .title(Span::styled(" Download ", theme.title));
    frame.render_widget(block, area);

    let inner_x = area.x + 2;
    let mut y = area.y + 1;

    let label = |text: &str, focused: bool| {
        Paragraph::new(Span::styled(
            text.to_string(),
            if focused { theme.help_key } else { theme.dim },
        ))
    };

    frame.render_widget(
        label("Repo (owner/repo or URL):", focus == FormFocus::Repo),
        Rect {
            x: inner_x,
            y,
            width: area.width.saturating_sub(4),
            height: 1,
        },
    );
    y += 1;
    repo_input.render(
        frame,
        Rect {
            x: inner_x,
            y,
            width: area.width.saturating_sub(4),
            height: 1,
        },
        if focus == FormFocus::Repo {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        },
    );
    y += 2;

    frame.render_widget(
        label("Target dir (blank = derive from repo):", focus == FormFocus::Dir),
        Rect {
            x: inner_x,
            y,
            width: area.width.saturating_sub(4),
            height: 1,
        },
    );
    y += 1;
    dir_input.render(
        frame,
        Rect {
            x: inner_x,
            y,
            width: area.width.saturating_sub(4),
            height: 1,
        },
        if focus == FormFocus::Dir {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        },
    );

    if let Some(err) = error {
        frame.render_widget(
            Paragraph::new(Span::styled(err.clone(), theme.error)),
            Rect {
                x: inner_x,
                y: area.y + area.height.saturating_sub(2),
                width: area.width.saturating_sub(4),
                height: 1,
            },
        );
    }
}

fn render_path_form(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    input: &TextInput,
    error: &Option<String>,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.brand)
        .title(Span::styled(format!(" {} ", title), theme.title));
    frame.render_widget(block, area);

    let inner_x = area.x + 2;
    let label = Paragraph::new(Span::styled("Path:", theme.help_key));
    frame.render_widget(
        label,
        Rect {
            x: inner_x,
            y: area.y + 1,
            width: area.width.saturating_sub(4),
            height: 1,
        },
    );
    input.render(
        frame,
        Rect {
            x: inner_x,
            y: area.y + 2,
            width: area.width.saturating_sub(4),
            height: 1,
        },
        Style::default().fg(Color::White),
    );

    if let Some(err) = error {
        frame.render_widget(
            Paragraph::new(Span::styled(err.clone(), theme.error)),
            Rect {
                x: inner_x,
                y: area.y + area.height.saturating_sub(2),
                width: area.width.saturating_sub(4),
                height: 1,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_target_dir_from_owner_repo() {
        assert_eq!(
            derive_target_dir("octocat/Hello-World"),
            PathBuf::from("Hello-World")
        );
    }

    #[test]
    fn derive_target_dir_strips_dot_git() {
        assert_eq!(
            derive_target_dir("https://github.com/octocat/Hello-World.git"),
            PathBuf::from("Hello-World")
        );
    }

    #[test]
    fn derive_target_dir_handles_ssh_form() {
        assert_eq!(
            derive_target_dir("git@github.com:octocat/Hello-World.git"),
            PathBuf::from("Hello-World")
        );
    }

    #[test]
    fn focus_toggles() {
        assert!(matches!(FormFocus::Repo.toggle(), FormFocus::Dir));
        assert!(matches!(FormFocus::Dir.toggle(), FormFocus::Repo));
    }
}
