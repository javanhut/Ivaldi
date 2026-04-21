//! TUI components for Ivaldi VCS.
//!
//! Built with `ratatui` + `crossterm` for interactive terminal UI:
//! - `travel` — browse history, diverge or overwrite
//! - `shift` — select commit range to squash
//! - `resolver` — per-file conflict resolution during fuse
//! - `app` — tabbed dashboard with status, log, diff, timelines, remote, fuse

pub mod app;
pub mod components;
pub mod config_form;
pub mod input;
pub mod resolver;
pub mod shift;
pub mod theme;
pub mod travel;
pub mod types;
pub mod views;

use crossterm::{
    ExecutableCommand,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use std::io;

/// Initialize the terminal for TUI mode.
pub fn init_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(io::stdout()))
}

/// Restore the terminal after TUI mode.
pub fn restore_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}
