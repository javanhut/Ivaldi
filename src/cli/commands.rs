//! Command implementations for Ivaldi CLI.
//!
//! Each function handles one CLI command, wiring user input to the core modules.
//! Commands that need persistent state use `Repo`; others use lightweight helpers.

use std::io::Write;
use std::path::PathBuf;
use std::process;

use crate::cas::FileCas;
use crate::color;
use crate::config::{self, Config};
use crate::forge;
use crate::fuse::Strategy;
use crate::ignore;
use crate::log as ivaldi_log;
use crate::portal::{Platform, Portal, PortalManager};
use crate::repo::Repo;
use crate::seal;
use crate::workspace::{FileState, Workspace};

use super::*;

mod dispatch;
pub use dispatch::run_command;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct RepoContext {
    work_dir: PathBuf,
    ivaldi_dir: PathBuf,
}

fn find_repo() -> Result<RepoContext, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = forge::find_repo_root(&cwd)
        .ok_or("not an Ivaldi repository (or any parent). Run 'ivaldi forge' to initialize.")?;
    Ok(RepoContext {
        ivaldi_dir: root.join(".ivaldi"),
        work_dir: root,
    })
}

fn open_repo() -> Result<Repo, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = forge::find_repo_root(&cwd)
        .ok_or("not an Ivaldi repository (or any parent). Run 'ivaldi forge' to initialize.")?;
    Repo::open(&root).map_err(|e| e.to_string())
}

mod fusing;
mod gather;
mod history;
mod inspect;
mod maintenance;
mod misc;
mod remote;
mod review;
mod sealing;
mod timelines;

use fusing::*;
use gather::*;
use history::*;
use inspect::*;
use maintenance::*;
use misc::*;
use remote::*;
use review::*;
use sealing::*;
use timelines::*;

#[cfg(test)]
mod tests;
