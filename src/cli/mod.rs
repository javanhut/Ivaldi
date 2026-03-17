//! CLI framework for Ivaldi VCS.
//!
//! All commands are defined using `clap` with 1:1 parity to the Go Cobra implementation.

mod commands;

use clap::{Parser, Subcommand};

pub use commands::run_command;

/// Ivaldi Version Control System
#[derive(Parser, Debug)]
#[command(name = "ivaldi", version = "0.1.0")]
#[command(about = "Ivaldi is a Version Control System")]
#[command(long_about = "Ivaldi is a VCS used to control repositories that can replace Git in your normal workflow")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Increase output verbosity (-v for info, -vv for debug)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress non-essential output
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize a new Ivaldi repository
    Forge,

    /// Stage files for the next seal
    Gather(GatherArgs),

    /// Create a sealed commit from staged files
    Seal(SealArgs),

    /// Show repository status
    Status,

    /// Show current timeline and position
    #[command(alias = "wai")]
    Whereami,

    /// View commit history
    Log(LogArgs),

    /// Compare changes
    Diff(DiffArgs),

    /// Unstage files or reset changes
    Reset(ResetArgs),

    /// Manage timelines (branches)
    #[command(alias = "tl")]
    Timeline(TimelineArgs),

    /// Merge timelines together
    Fuse(FuseArgs),

    /// Interactive time travel through history
    Travel(TravelArgs),

    /// Squash commits interactively
    Shift(ShiftArgs),

    /// View and modify configuration
    Config(ConfigArgs),

    /// Add patterns to .ivaldiignore
    Exclude(ExcludeArgs),

    /// Manage GitHub/GitLab repository connections
    Portal(PortalArgs),

    /// Authenticate with GitHub/GitLab
    Auth(AuthArgs),

    /// Clone a repository from GitHub/GitLab
    Download(DownloadArgs),

    /// Push commits to GitHub/GitLab
    Upload(UploadArgs),

    /// Discover remote timelines
    Scout(ScoutArgs),

    /// Download specific remote timelines
    Harvest(HarvestArgs),
}

// ---------------------------------------------------------------------------
// Argument structs
// ---------------------------------------------------------------------------

#[derive(clap::Args, Debug)]
pub struct GatherArgs {
    /// Files to stage (or "." for all)
    #[arg(num_args = 0..)]
    pub files: Vec<String>,

    /// Skip interactive prompts for hidden files
    #[arg(long)]
    pub allow_all: bool,
}

#[derive(clap::Args, Debug)]
pub struct SealArgs {
    /// Commit message
    #[arg()]
    pub message: Option<String>,

    /// Commit message (alternative flag)
    #[arg(short)]
    pub m: Option<String>,
}

impl SealArgs {
    pub fn get_message(&self) -> Option<&str> {
        self.message
            .as_deref()
            .or(self.m.as_deref())
    }
}

#[derive(clap::Args, Debug)]
pub struct LogArgs {
    /// Show concise one-line format
    #[arg(long)]
    pub oneline: bool,

    /// Limit number of commits shown
    #[arg(long)]
    pub limit: Option<usize>,

    /// Show commits from all timelines
    #[arg(long)]
    pub all: bool,
}

#[derive(clap::Args, Debug)]
pub struct DiffArgs {
    /// Show staged changes
    #[arg(long)]
    pub staged: bool,

    /// Show summary statistics only
    #[arg(long)]
    pub stat: bool,

    /// Seal name or hash prefix to compare against
    #[arg()]
    pub target: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct ResetArgs {
    /// Files to unstage
    #[arg(num_args = 0..)]
    pub files: Vec<String>,

    /// Discard all uncommitted changes (destructive!)
    #[arg(long)]
    pub hard: bool,
}

#[derive(clap::Args, Debug)]
pub struct TimelineArgs {
    #[command(subcommand)]
    pub command: TimelineCommands,
}

#[derive(Subcommand, Debug)]
pub enum TimelineCommands {
    /// Create a new timeline
    Create(TimelineCreateArgs),

    /// Switch to a different timeline
    #[command(alias = "sw")]
    Switch(TimelineSwitchArgs),

    /// List all timelines
    #[command(alias = "ls")]
    List,

    /// Remove a timeline
    #[command(alias = "rm")]
    Remove(TimelineRemoveArgs),

    /// Manage butterfly (experimental) timelines
    #[command(alias = "bf")]
    Butterfly(ButterflyArgs),
}

#[derive(clap::Args, Debug)]
pub struct TimelineCreateArgs {
    /// Name for the new timeline
    pub name: String,
    /// Source timeline (defaults to current)
    pub from: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct TimelineSwitchArgs {
    /// Timeline to switch to
    pub name: String,
}

#[derive(clap::Args, Debug)]
pub struct TimelineRemoveArgs {
    /// Timeline to remove
    pub name: String,
}

#[derive(clap::Args, Debug)]
pub struct ButterflyArgs {
    #[command(subcommand)]
    pub command: ButterflyCommands,
}

#[derive(Subcommand, Debug)]
pub enum ButterflyCommands {
    /// Create a new butterfly timeline
    Create(ButterflyCreateArgs),

    /// Sync changes up to parent timeline
    Up,

    /// Sync changes down from parent timeline
    Down,

    /// Remove a butterfly timeline
    #[command(alias = "rm")]
    Remove(ButterflyRemoveArgs),
}

#[derive(clap::Args, Debug)]
pub struct ButterflyCreateArgs {
    /// Name for the butterfly timeline
    pub name: String,
}

#[derive(clap::Args, Debug)]
pub struct ButterflyRemoveArgs {
    /// Butterfly to remove
    pub name: String,

    /// Recursively delete nested butterflies
    #[arg(long)]
    pub cascade: bool,
}

#[derive(clap::Args, Debug)]
pub struct FuseArgs {
    /// Source timeline
    pub source: Option<String>,

    /// Literal "to" keyword (consumed but ignored)
    #[arg(hide = true)]
    pub to_keyword: Option<String>,

    /// Target timeline
    pub target: Option<String>,

    /// Merge strategy (auto, ours, theirs, union, base)
    #[arg(long, default_value = "auto")]
    pub strategy: String,

    /// Continue merge after resolving conflicts
    #[arg(long, name = "continue")]
    pub continue_merge: bool,

    /// Abort current merge
    #[arg(long)]
    pub abort: bool,
}

#[derive(clap::Args, Debug)]
pub struct TravelArgs {
    /// Number of seals in viewport (0 for auto-detect)
    #[arg(short = 'w', long, default_value = "0")]
    pub window_size: usize,

    /// Filter seals by message, author, or name
    #[arg(short, long)]
    pub search: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct ShiftArgs {
    /// Squash last N commits
    #[arg(long)]
    pub last: Option<usize>,

    /// Start seal name or hash
    pub start: Option<String>,

    /// End seal name or hash
    pub end: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct ConfigArgs {
    /// List all configuration values
    #[arg(long)]
    pub list: bool,

    /// Set a configuration value
    #[arg(long)]
    pub set: Option<String>,

    /// Get a configuration value
    #[arg(long)]
    pub get: Option<String>,

    /// Value for --set
    pub value: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct ExcludeArgs {
    /// Patterns to add to .ivaldiignore
    #[arg(required = true, num_args = 1..)]
    pub patterns: Vec<String>,
}

#[derive(clap::Args, Debug)]
pub struct PortalArgs {
    #[command(subcommand)]
    pub command: PortalCommands,
}

#[derive(Subcommand, Debug)]
pub enum PortalCommands {
    /// Add a remote repository connection
    Add(PortalAddArgs),

    /// List configured portals
    List,

    /// Remove a portal
    Remove(PortalRemoveArgs),
}

#[derive(clap::Args, Debug)]
pub struct PortalAddArgs {
    /// Repository in owner/repo format
    pub repo: String,

    /// Use GitLab instead of GitHub
    #[arg(long)]
    pub gitlab: bool,

    /// Custom instance URL (for self-hosted)
    #[arg(long)]
    pub url: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct PortalRemoveArgs {
    /// Repository to remove (owner/repo)
    pub repo: String,
}

#[derive(clap::Args, Debug)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub command: AuthCommands,
}

#[derive(Subcommand, Debug)]
pub enum AuthCommands {
    /// Authenticate with a platform
    Login(AuthLoginArgs),

    /// Show authentication status
    Status,

    /// Remove stored credentials
    Logout(AuthLogoutArgs),
}

#[derive(clap::Args, Debug)]
pub struct AuthLoginArgs {
    /// Authenticate with GitLab instead of GitHub
    #[arg(long)]
    pub gitlab: bool,
}

#[derive(clap::Args, Debug)]
pub struct AuthLogoutArgs {
    /// Log out from GitLab instead of GitHub
    #[arg(long)]
    pub gitlab: bool,
}

#[derive(clap::Args, Debug)]
pub struct DownloadArgs {
    /// Repository (owner/repo or URL)
    pub repo: String,

    /// Target directory
    pub directory: Option<String>,

    /// Limit commit history depth
    #[arg(long, default_value = "0")]
    pub depth: usize,

    /// Skip history, download only latest snapshot
    #[arg(long)]
    pub skip_history: bool,

    /// Include tags and releases
    #[arg(long)]
    pub include_tags: bool,

    /// Use GitLab
    #[arg(long)]
    pub gitlab: bool,

    /// Custom instance URL
    #[arg(long)]
    pub url: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct UploadArgs {
    /// Branch name (defaults to current timeline)
    pub branch: Option<String>,

    /// Force push (overwrites remote history)
    #[arg(long)]
    pub force: bool,
}

#[derive(clap::Args, Debug)]
pub struct ScoutArgs {
    /// Force refresh of remote information
    #[arg(long)]
    pub refresh: bool,
}

#[derive(clap::Args, Debug)]
pub struct HarvestArgs {
    /// Specific timelines to download
    #[arg(num_args = 0..)]
    pub timelines: Vec<String>,

    /// Update existing timelines and download new ones
    #[arg(long)]
    pub update: bool,
}
