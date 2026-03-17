//! Progress bars and spinners for Ivaldi VCS.
//!
//! Uses `indicatif` for:
//! - Download/upload file progress
//! - Commit processing progress
//! - Indeterminate spinners for waiting operations

use indicatif::{ProgressBar, ProgressStyle};

/// Create a progress bar for file operations (download/upload).
pub fn file_bar(total: u64, action: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(&format!(
                "{{spinner:.green}} {} [{{bar:30.cyan/blue}}] {{pos}}/{{len}} ({{eta}})",
                action
            ))
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("█▓░"),
    );
    pb
}

/// Create a progress bar for commit processing.
pub fn commit_bar(total: u64) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} Processing commits [{bar:30.yellow/red}] {pos}/{len}")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("█▓░"),
    );
    pb
}

/// Create an indeterminate spinner.
pub fn spinner(message: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb
}

/// Create a byte-count progress bar (for large downloads).
pub fn byte_bar(total: u64, action: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(&format!(
                "{{spinner:.green}} {} [{{bar:30.cyan/blue}}] {{bytes}}/{{total_bytes}} ({{bytes_per_sec}})",
                action
            ))
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("█▓░"),
    );
    pb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_file_bar() {
        let pb = file_bar(100, "Downloading");
        pb.inc(50);
        assert_eq!(pb.position(), 50);
        pb.finish();
    }

    #[test]
    fn create_spinner() {
        let pb = spinner("Working...");
        pb.finish_with_message("Done");
    }

    #[test]
    fn create_commit_bar() {
        let pb = commit_bar(10);
        for _ in 0..10 { pb.inc(1); }
        pb.finish();
    }
}
