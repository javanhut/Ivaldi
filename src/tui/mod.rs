//! TUI components for Ivaldi VCS.
//!
//! Built with `ratatui` + `crossterm` for interactive terminal UI:
//! - `travel` — browse history, diverge or overwrite
//! - `shift` — select commit range to squash
//! - `resolver` — per-file conflict resolution during fuse

pub mod travel;
pub mod shift;
pub mod resolver;

use std::io;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;

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
