//! Structured logging for Ivaldi VCS.
//!
//! Provides leveled logging controlled by the `-v` flag:
//! - No flag: errors only
//! - `-v`: info + errors
//! - `-vv`: debug + info + errors
//!
//! Respects `--quiet` to suppress non-error output.

use std::sync::atomic::{AtomicU8, Ordering};

static VERBOSITY: AtomicU8 = AtomicU8::new(0);
static QUIET: AtomicBool = AtomicBool::new(false);

use std::sync::atomic::AtomicBool;

/// Log levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
}

/// Initialize logging with the given verbosity (from `-v` count) and quiet flag.
pub fn init(verbosity: u8, quiet: bool) {
    VERBOSITY.store(verbosity, Ordering::Relaxed);
    QUIET.store(quiet, Ordering::Relaxed);
}

/// Get current verbosity level.
pub fn verbosity() -> u8 {
    VERBOSITY.load(Ordering::Relaxed)
}

/// Check if quiet mode is enabled.
pub fn is_quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

/// Check if a given level should be logged.
pub fn should_log(level: Level) -> bool {
    if is_quiet() && level != Level::Error {
        return false;
    }
    let v = verbosity();
    match level {
        Level::Error => true,
        Level::Warn => v >= 1,
        Level::Info => v >= 1,
        Level::Debug => v >= 2,
    }
}

/// Log an error message (always shown).
pub fn error(msg: &str) {
    if should_log(Level::Error) {
        eprintln!("{} {}", crate::color::bold_red("[ERROR]"), msg);
    }
}

/// Log a warning message (shown with -v or higher).
pub fn warn(msg: &str) {
    if should_log(Level::Warn) {
        eprintln!("{} {}", crate::color::bold_yellow("[WARN]"), msg);
    }
}

/// Log an info message (shown with -v or higher).
pub fn info(msg: &str) {
    if should_log(Level::Info) {
        eprintln!("{} {}", crate::color::blue("[INFO]"), msg);
    }
}

/// Log a debug message (shown with -vv).
pub fn debug(msg: &str) {
    if should_log(Level::Debug) {
        eprintln!("{} {}", crate::color::dim("[DEBUG]"), msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_verbosity() {
        init(0, false);
        assert!(should_log(Level::Error));
        assert!(!should_log(Level::Info));
        assert!(!should_log(Level::Debug));
    }

    #[test]
    fn verbose_once() {
        init(1, false);
        assert!(should_log(Level::Error));
        assert!(should_log(Level::Info));
        assert!(should_log(Level::Warn));
        assert!(!should_log(Level::Debug));
    }

    #[test]
    fn verbose_twice() {
        init(2, false);
        assert!(should_log(Level::Debug));
    }

    #[test]
    fn quiet_mode() {
        init(2, true);
        assert!(should_log(Level::Error)); // errors always
        assert!(!should_log(Level::Info)); // suppressed
        assert!(!should_log(Level::Debug)); // suppressed
    }
}
