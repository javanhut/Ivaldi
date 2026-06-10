//! Colored terminal output for Ivaldi VCS.
//!
//! Uses ANSI escape codes directly — no extra dependency needed.
//! Respects `NO_COLOR` env var and `color.ui` config.

use std::sync::atomic::{AtomicBool, Ordering};

static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);

/// Set whether color output is enabled globally.
pub fn set_enabled(enabled: bool) {
    COLOR_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Check if color output is enabled.
pub fn is_enabled() -> bool {
    COLOR_ENABLED.load(Ordering::Relaxed)
}

/// Initialize color from environment. Call once at startup.
pub fn init() {
    if std::env::var("NO_COLOR").is_ok() {
        set_enabled(false);
    }
    // Also check if stdout is a terminal
    if !atty_stdout() {
        set_enabled(false);
    }
}

fn atty_stdout() -> bool {
    // Simple heuristic — check if TERM is set
    std::env::var("TERM").is_ok()
}

// ANSI color codes
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";

fn wrap(code: &str, text: &str) -> String {
    if is_enabled() {
        format!("{}{}{}", code, text, RESET)
    } else {
        text.to_string()
    }
}

pub fn bold(text: &str) -> String {
    wrap(BOLD, text)
}
pub fn dim(text: &str) -> String {
    wrap(DIM, text)
}
pub fn red(text: &str) -> String {
    wrap(RED, text)
}
pub fn green(text: &str) -> String {
    wrap(GREEN, text)
}
pub fn yellow(text: &str) -> String {
    wrap(YELLOW, text)
}
pub fn blue(text: &str) -> String {
    wrap(BLUE, text)
}
pub fn magenta(text: &str) -> String {
    wrap(MAGENTA, text)
}
pub fn cyan(text: &str) -> String {
    wrap(CYAN, text)
}

/// Render a CLI error line with a consistent red `error:` prefix.
pub fn error(msg: &str) -> String {
    format!("{} {}", bold_red("error:"), msg)
}

pub fn bold_green(text: &str) -> String {
    if is_enabled() {
        format!("{}{}{}{}", BOLD, GREEN, text, RESET)
    } else {
        text.to_string()
    }
}
pub fn bold_red(text: &str) -> String {
    if is_enabled() {
        format!("{}{}{}{}", BOLD, RED, text, RESET)
    } else {
        text.to_string()
    }
}
pub fn bold_yellow(text: &str) -> String {
    if is_enabled() {
        format!("{}{}{}{}", BOLD, YELLOW, text, RESET)
    } else {
        text.to_string()
    }
}
pub fn bold_cyan(text: &str) -> String {
    if is_enabled() {
        format!("{}{}{}{}", BOLD, CYAN, text, RESET)
    } else {
        text.to_string()
    }
}

/// Format a file state for status display.
pub fn status_label(state: &str) -> String {
    match state {
        "staged" => green("staged"),
        "modified" => yellow("modified"),
        "deleted" => red("deleted"),
        "added" => green("added"),
        "untracked" => dim("untracked"),
        "conflict" => bold_red("CONFLICT"),
        _ => state.to_string(),
    }
}

/// Format a seal name with color.
pub fn seal_name(name: &str) -> String {
    bold_cyan(name)
}

/// Format a hash with color.
pub fn hash(h: &str) -> String {
    yellow(h)
}

/// Format a timeline name.
pub fn timeline(name: &str) -> String {
    bold_green(name)
}

/// Format an author.
pub fn author(name: &str) -> String {
    blue(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_toggle() {
        set_enabled(true);
        assert!(is_enabled());
        let colored = red("error");
        assert!(colored.contains("\x1b[31m"));

        set_enabled(false);
        let plain = red("error");
        assert_eq!(plain, "error");

        set_enabled(true); // restore
    }

    #[test]
    fn status_labels() {
        set_enabled(false);
        assert_eq!(status_label("staged"), "staged");
        assert_eq!(status_label("modified"), "modified");
        set_enabled(true);
    }

    #[test]
    fn no_color_env() {
        // Can't easily test env vars in parallel, just verify init doesn't panic
        init();
    }
}
