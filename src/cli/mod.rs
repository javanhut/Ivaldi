//! CLI framework for Ivaldi VCS.
//!
//! All commands are defined using `clap` with 1:1 parity to the Go Cobra implementation.

mod commands;
mod json;

use clap::{Parser, Subcommand};

pub use commands::run_command;

/// Ivaldi Version Control System
#[derive(Parser, Debug)]
#[command(name = "ivaldi", version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Ivaldi is a Version Control System")]
#[command(
    long_about = "Ivaldi is a VCS used to control repositories that can replace Git in your normal workflow"
)]
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
    #[command(alias = "init")]
    Forge,

    /// Stage files for the next seal
    #[command(alias = "add")]
    Gather(GatherArgs),

    /// Create a sealed commit from staged files
    #[command(alias = "se")]
    Seal(SealArgs),

    /// Redo the most recent seal, folding in staged changes and/or a new message
    #[command(alias = "rs")]
    Reseal(ResealArgs),

    /// Show repository status
    #[command(alias = "st")]
    Status(StatusArgs),

    /// Show current timeline and position
    #[command(alias = "wai")]
    Whereami,

    /// View commit history
    #[command(alias = "lg")]
    Log(LogArgs),

    /// Show, line-by-line, which seal last touched each line of a file
    #[command(aliases = ["blame", "wd"])]
    Whodidit(WhodiditArgs),

    /// Compare changes
    #[command(alias = "df")]
    Diff(DiffArgs),

    /// Remove files from the gathered set (unstage)
    #[command(alias = "dc")]
    Discard(DiscardArgs),

    /// Throw away all uncommitted changes and restore the working
    /// directory from the last seal (destructive!)
    Reverse(ReverseArgs),

    /// Move the timeline head back to an earlier seal
    #[command(alias = "rw")]
    Rewind(RewindArgs),

    /// Create a seal that undoes an earlier seal's changes
    #[command(alias = "ud")]
    Undo(UndoArgs),

    /// Apply another seal's changes to the current timeline
    #[command(aliases = ["cherry-pick", "pl"])]
    Pluck(PluckArgs),

    /// Manage timelines (branches)
    #[command(alias = "tl")]
    Timeline(TimelineArgs),

    /// Merge timelines together
    #[command(alias = "fu")]
    Fuse(FuseArgs),

    /// Interactive time travel through history
    #[command(alias = "tv")]
    Travel(TravelArgs),

    /// Combine a range of seals into a single seal (linear history)
    #[command(alias = "w")]
    Weld(WeldArgs),

    /// View and modify configuration (bare `config` opens an interactive form)
    #[command(aliases = ["configure", "cf"])]
    Config(ConfigArgs),

    /// Add patterns to .ivaldiignore
    #[command(alias = "ex")]
    Exclude(ExcludeArgs),

    /// Manage GitHub/GitLab repository connections
    #[command(alias = "pt")]
    Portal(PortalArgs),

    /// Authenticate with GitHub/GitLab
    #[command(alias = "au")]
    Auth(AuthArgs),

    /// Clone a repository from GitHub/GitLab
    #[command(alias = "dl")]
    Download(DownloadArgs),

    /// Push commits to GitHub/GitLab
    #[command(alias = "up")]
    Upload(UploadArgs),

    /// Discover remote timelines
    #[command(alias = "sc")]
    Scout(ScoutArgs),

    /// Download specific remote timelines
    #[command(alias = "hv")]
    Harvest(HarvestArgs),

    /// Sync current timeline with remote (delta only)
    #[command(alias = "sy")]
    Sync(SyncArgs),

    /// Local code review system
    #[command(alias = "rv")]
    Review(ReviewArgs),

    /// Open interactive TUI dashboard
    #[command(alias = "ui")]
    Tui,

    /// Serve the current repo to authorized peers over `ivaldi://`
    #[command(alias = "sv")]
    Serve(ServeArgs),

    /// Manage authorized peers for `ivaldi serve`
    #[command(alias = "pr")]
    Peer(PeerArgs),

    /// Generate shell completion script to stdout
    #[command(alias = "cmp")]
    Completions(CompletionsArgs),

    /// Generate man pages into a directory
    #[command(hide = true)]
    Man(ManArgs),

    /// Check repository integrity
    Verify(VerifyArgs),
}

#[derive(clap::Args, Debug)]
pub struct VerifyArgs {
    /// Re-hash every stored object (slower; O(total repository size)).
    #[arg(long)]
    pub full: bool,

    /// Emit the report as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Debug)]
pub struct UndoArgs {
    /// Seal to undo (name or hash prefix)
    pub seal: String,

    /// Custom message (default: Undo "<original first line>")
    #[arg(short)]
    pub m: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct PluckArgs {
    /// Seal to apply (name or hash prefix)
    pub seal: String,

    /// Custom message (default: original message with a "plucked from" trailer)
    #[arg(short)]
    pub m: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct CompletionsArgs {
    /// Shell to generate completions for
    pub shell: CompletionShell,
}

/// Shells that `ivaldi completions` can target.
///
/// The standard shells are rendered by `clap_complete`; `raven` emits a
/// RavenShell JSON completion spec (the file that belongs at
/// `~/.config/ravenshell/completions/ivaldi.json`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum CompletionShell {
    /// Bourne Again SHell
    Bash,
    /// Elvish shell
    Elvish,
    /// Friendly Interactive SHell
    Fish,
    /// PowerShell
    #[value(name = "powershell")]
    PowerShell,
    /// Z SHell
    Zsh,
    /// RavenShell (JSON completion spec)
    Raven,
}

impl CompletionShell {
    /// The matching `clap_complete` shell, or `None` for RavenShell, whose
    /// spec is generated separately.
    pub fn clap_shell(self) -> Option<clap_complete::Shell> {
        match self {
            CompletionShell::Bash => Some(clap_complete::Shell::Bash),
            CompletionShell::Elvish => Some(clap_complete::Shell::Elvish),
            CompletionShell::Fish => Some(clap_complete::Shell::Fish),
            CompletionShell::PowerShell => Some(clap_complete::Shell::PowerShell),
            CompletionShell::Zsh => Some(clap_complete::Shell::Zsh),
            CompletionShell::Raven => None,
        }
    }
}

#[derive(clap::Args, Debug)]
pub struct ManArgs {
    /// Output directory for the generated man pages
    #[arg(long, default_value = "man")]
    pub out: std::path::PathBuf,
}

#[derive(clap::Args, Debug)]
pub struct ServeArgs {
    /// Address to bind (default 0.0.0.0:9418)
    #[arg(long, default_value = "0.0.0.0:9418")]
    pub bind: String,
}

#[derive(clap::Args, Debug)]
pub struct PeerArgs {
    #[command(subcommand)]
    pub command: PeerCommands,
}

#[derive(Subcommand, Debug)]
pub enum PeerCommands {
    /// Trust a peer's pubkey for incoming `ivaldi serve` connections
    #[command(alias = "tr")]
    Trust(PeerTrustArgs),
    /// List trusted peers
    #[command(alias = "ls")]
    List,
    /// Remove a trusted peer by pubkey-hex prefix
    #[command(alias = "fg")]
    Forget(PeerForgetArgs),
    /// Print this user's own pubkey
    #[command(alias = "wi")]
    Whoami,
    /// Manage TOFU `~/.ivaldi/known_peers` (servers we connect to)
    #[command(alias = "kn")]
    Known(PeerKnownArgs),
}

#[derive(clap::Args, Debug)]
pub struct PeerKnownArgs {
    #[command(subcommand)]
    pub command: PeerKnownCommands,
}

#[derive(Subcommand, Debug)]
pub enum PeerKnownCommands {
    /// List known peers (host:port → pubkey)
    #[command(alias = "ls")]
    List,
    /// Forget a known peer by host[:port]
    #[command(alias = "fg")]
    Forget(PeerKnownForgetArgs),
}

#[derive(clap::Args, Debug)]
pub struct PeerKnownForgetArgs {
    /// Host or host:port to forget. Defaults to the ivaldi default port.
    pub host: String,
}

#[derive(clap::Args, Debug)]
pub struct PeerTrustArgs {
    /// Hex-encoded 32-byte X25519 pubkey
    pub pubkey: String,
    /// Optional friendly name
    pub name: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct PeerForgetArgs {
    /// Pubkey or unique hex prefix
    pub prefix: String,
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

    /// Interactively choose which hunks of each changed file to stage
    #[arg(short = 'p', long, conflicts_with = "allow_all")]
    pub patch: bool,
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

#[derive(clap::Args, Debug)]
pub struct ResealArgs {
    /// New message (without one, the old message is kept)
    #[arg()]
    pub message: Option<String>,

    /// New message (alternative flag)
    #[arg(short)]
    pub m: Option<String>,
}

impl ResealArgs {
    pub fn get_message(&self) -> Option<&str> {
        self.message.as_deref().or(self.m.as_deref())
    }
}

impl SealArgs {
    pub fn get_message(&self) -> Option<&str> {
        self.message.as_deref().or(self.m.as_deref())
    }
}

#[derive(clap::Args, Debug)]
pub struct StatusArgs {
    /// Emit machine-readable JSON instead of human-readable output
    #[arg(long)]
    pub json: bool,
}

/// Output format for `ivaldi log`.
#[derive(Clone, Copy, Debug, PartialEq, clap::ValueEnum)]
pub enum LogFormat {
    /// One line per seal (same as --oneline)
    Short,
    /// Default multi-line format
    Medium,
    /// Medium plus absolute date, tree root, and merge parents
    Full,
    /// Machine-readable JSON array
    Json,
}

#[derive(clap::Args, Debug)]
pub struct LogArgs {
    /// Show concise one-line format
    #[arg(long)]
    pub oneline: bool,

    /// Output format (short, medium, full, json)
    #[arg(long, value_enum, conflicts_with = "oneline")]
    pub format: Option<LogFormat>,

    /// Limit number of commits shown
    #[arg(long)]
    pub limit: Option<usize>,

    /// Show commits from all timelines
    #[arg(long)]
    pub all: bool,
}

#[derive(clap::Args, Debug)]
pub struct WhodiditArgs {
    /// File to inspect, relative to the repository root.
    pub path: String,

    /// Show only the seal/author summary (one line per file region) rather
    /// than every individual line.
    #[arg(long)]
    pub summary: bool,
}

#[derive(clap::Args, Debug)]
pub struct DiffArgs {
    /// Show staged changes
    #[arg(long)]
    pub staged: bool,

    /// Show summary statistics only
    #[arg(long)]
    pub stat: bool,

    /// Timeline names, seal names, or hash prefixes to compare
    #[arg(num_args = 0..=2)]
    pub targets: Vec<String>,
}

#[derive(clap::Args, Debug)]
pub struct DiscardArgs {
    /// Files to remove from the gathered set (none = everything)
    #[arg(num_args = 0..)]
    pub files: Vec<String>,
}

#[derive(clap::Args, Debug)]
pub struct ReverseArgs {
    /// Required confirmation that every uncommitted change should go
    #[arg(long, required = true)]
    pub all: bool,
}

#[derive(clap::Args, Debug)]
pub struct RewindArgs {
    /// Seal to rewind the timeline head to (name or hash prefix)
    pub seal: String,

    /// Also rewrite the working directory to match that seal (destructive!).
    /// Without this flag your files are left exactly as they are.
    #[arg(long)]
    pub discard: bool,
}

#[derive(clap::Args, Debug)]
pub struct TimelineArgs {
    #[command(subcommand)]
    pub command: TimelineCommands,
}

#[derive(Subcommand, Debug)]
pub enum TimelineCommands {
    /// Create a new timeline and switch to it
    #[command(alias = "cr")]
    Create(TimelineCreateArgs),

    /// Switch to a different timeline
    #[command(alias = "sw")]
    Switch(TimelineSwitchArgs),

    /// List all timelines
    #[command(alias = "ls")]
    List(TimelineListArgs),

    /// Remove a timeline
    #[command(alias = "rm")]
    Remove(TimelineRemoveArgs),

    /// Rename the current timeline
    #[command(alias = "rn")]
    Rename(TimelineRenameArgs),

    /// Manage butterfly (experimental) timelines
    #[command(alias = "bf")]
    Butterfly(ButterflyArgs),
}

#[derive(clap::Args, Debug)]
pub struct TimelineListArgs {
    /// Emit machine-readable JSON instead of human-readable output
    #[arg(long)]
    pub json: bool,
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
pub struct TimelineRenameArgs {
    /// One of three forms:
    ///
    ///   `tl rename NEW`             — rename the current timeline to NEW
    ///   `tl rename OLD NEW`         — rename OLD to NEW
    ///   `tl rename OLD to NEW`      — same as above with `to` as a connector
    #[arg(num_args = 1..=3)]
    pub names: Vec<String>,
}

#[derive(clap::Args, Debug)]
pub struct ButterflyArgs {
    #[command(subcommand)]
    pub command: ButterflyCommands,
}

#[derive(Subcommand, Debug)]
pub enum ButterflyCommands {
    /// Create a new butterfly timeline
    #[command(alias = "cr")]
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

    /// Browse every seal in the MMR — including ones orphaned from the
    /// current timeline head (e.g., commits welded out of the chain).
    /// Without this flag, travel only shows seals reachable from HEAD.
    #[arg(long)]
    pub all: bool,
}

#[derive(clap::Args, Debug)]
pub struct WeldArgs {
    /// Combine the last N seals on the current timeline.
    #[arg(long)]
    pub last: Option<usize>,

    /// Range start: first seal to include (oldest). Seal name or hash prefix.
    /// Optional connector `to` is accepted between START and END for ergonomics:
    ///   `ivaldi weld bold-tower to clear-galaxy`
    pub start: Option<String>,

    /// Either the literal `to` (connector) or the END seal of the range.
    pub second: Option<String>,

    /// Range end: last seal to include (newest, defaults to current head).
    /// Only used when the connector form `START to END` is given.
    pub end: Option<String>,

    /// Message for the welded seal. If omitted, a summary of the welded
    /// seals' messages is generated.
    #[arg(short)]
    pub m: Option<String>,
}

#[derive(clap::Args, Debug)]
#[command(after_help = format!(
    "Known keys:\n{}\n\nExamples:\n  ivaldi config --set user.name \"Ada Lovelace\"\n  ivaldi config --set user.email ada@example.com\n  ivaldi config --global --set color.ui false\n  ivaldi config --list\n  ivaldi config            (interactive form with local/global selector)",
    crate::config::known_keys_help()
))]
pub struct ConfigArgs {
    /// List all configuration values
    #[arg(long)]
    pub list: bool,

    /// Set a configuration value (key as argument, value positional)
    #[arg(long, value_name = "KEY")]
    pub set: Option<String>,

    /// Get a configuration value
    #[arg(long, value_name = "KEY")]
    pub get: Option<String>,

    /// Operate on the global config (~/.ivaldi/config) instead of repo-local
    #[arg(long)]
    pub global: bool,

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
    #[command(alias = "ad")]
    Add(PortalAddArgs),

    /// List configured portals
    #[command(alias = "ls")]
    List(PortalListArgs),

    /// Remove a portal
    #[command(alias = "rm")]
    Remove(PortalRemoveArgs),
}

#[derive(clap::Args, Debug)]
pub struct PortalListArgs {
    /// Emit machine-readable JSON instead of human-readable output
    #[arg(long)]
    pub json: bool,
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
    #[command(alias = "li")]
    Login(AuthLoginArgs),

    /// Show authentication status
    #[command(alias = "st")]
    Status,

    /// Remove stored credentials
    #[command(alias = "lo")]
    Logout(AuthLogoutArgs),
}

#[derive(clap::Args, Debug)]
pub struct AuthLoginArgs {
    /// Authenticate with GitLab instead of GitHub
    #[arg(long)]
    pub gitlab: bool,

    /// GitLab base URL (defaults to https://gitlab.com or `IVALDI_GITLAB_HOST`).
    /// Only meaningful with `--gitlab`.
    #[arg(long)]
    pub gitlab_host: Option<String>,

    /// Read a Personal Access Token from stdin and store it, instead of
    /// running the browser device flow. PATs are independent per device and
    /// are NOT subject to GitHub's 10-token-per-OAuth-app eviction, so this is
    /// the most reliable choice when you use ivaldi across many machines.
    #[arg(long)]
    pub with_token: bool,

    /// Authenticate even when a usable credential (gh CLI / env var / an
    /// existing login) is already present, minting a separate ivaldi token.
    #[arg(long)]
    pub force: bool,
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

    /// (ivaldi:// only) Auto-trust the server's pubkey on first connect
    /// instead of prompting. Useful for scripts/CI.
    #[arg(long)]
    pub accept_new_peer: bool,

    /// (ivaldi:// only) Refuse to connect to any pubkey not already in
    /// `~/.ivaldi/known_peers`. Mutually exclusive with `--accept-new-peer`.
    #[arg(long, conflicts_with = "accept_new_peer")]
    pub strict_peer: bool,

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

#[derive(clap::Args, Debug)]
pub struct SyncArgs {
    /// Timeline to sync (defaults to current)
    pub timeline: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct ReviewArgs {
    #[command(subcommand)]
    pub command: ReviewCommands,
}

#[derive(Subcommand, Debug)]
pub enum ReviewCommands {
    /// Create a new review
    #[command(alias = "cr")]
    Create(ReviewCreateArgs),

    /// List reviews
    #[command(alias = "ls")]
    List(ReviewListArgs),

    /// Show review details
    #[command(alias = "sh")]
    Show(ReviewShowArgs),

    /// Show diff for a review
    #[command(alias = "df")]
    Diff(ReviewDiffArgs),

    /// Add a comment to a review
    #[command(alias = "cm")]
    Comment(ReviewCommentArgs),

    /// Approve a review
    #[command(alias = "ap")]
    Approve(ReviewApproveArgs),

    /// Request changes on a review
    #[command(alias = "rc")]
    RequestChanges(ReviewRequestChangesArgs),

    /// Merge an approved review
    #[command(alias = "mg")]
    Merge(ReviewMergeArgs),

    /// Close a review without merging
    #[command(alias = "cl")]
    Close(ReviewCloseArgs),

    /// Reopen a closed review
    #[command(alias = "ro")]
    Reopen(ReviewReopenArgs),
}

#[derive(clap::Args, Debug)]
pub struct ReviewCreateArgs {
    /// Source timeline
    #[arg(long)]
    pub source: String,

    /// Target timeline
    #[arg(long, default_value = "main")]
    pub target: String,

    /// Review title
    #[arg(long)]
    pub title: String,

    /// Review description
    #[arg(long, default_value = "")]
    pub description: String,

    /// Fuse strategy (auto, ours, theirs, union, base)
    #[arg(long, default_value = "auto")]
    pub strategy: String,
}

#[derive(clap::Args, Debug)]
pub struct ReviewListArgs {
    /// Filter by status
    #[arg(long)]
    pub status: Option<String>,

    /// Show all reviews (including merged/closed)
    #[arg(long)]
    pub all: bool,
}

#[derive(clap::Args, Debug)]
pub struct ReviewShowArgs {
    /// Review ID
    pub id: u64,
}

#[derive(clap::Args, Debug)]
pub struct ReviewDiffArgs {
    /// Review ID
    pub id: u64,

    /// Show summary statistics only
    #[arg(long)]
    pub stat: bool,
}

#[derive(clap::Args, Debug)]
pub struct ReviewCommentArgs {
    /// Review ID
    pub id: u64,

    /// File to comment on
    #[arg(long)]
    pub file: String,

    /// Line number (omit for file-level comment)
    #[arg(long)]
    pub line: Option<u64>,

    /// Comment body
    #[arg(long)]
    pub body: String,

    /// Reply to a specific comment ID
    #[arg(long)]
    pub reply_to: Option<u64>,
}

#[derive(clap::Args, Debug)]
pub struct ReviewApproveArgs {
    /// Review ID
    pub id: u64,

    /// Optional approval message
    #[arg(long, default_value = "")]
    pub body: String,
}

#[derive(clap::Args, Debug)]
pub struct ReviewRequestChangesArgs {
    /// Review ID
    pub id: u64,

    /// Reason for requesting changes
    #[arg(long)]
    pub body: String,
}

#[derive(clap::Args, Debug)]
pub struct ReviewMergeArgs {
    /// Review ID
    pub id: u64,

    /// Override fuse strategy
    #[arg(long)]
    pub strategy: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct ReviewCloseArgs {
    /// Review ID
    pub id: u64,
}

#[derive(clap::Args, Debug)]
pub struct ReviewReopenArgs {
    /// Review ID
    pub id: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parse_configure_alias() {
        let cli = Cli::try_parse_from(["ivaldi", "configure"]).unwrap();
        assert!(matches!(cli.command.unwrap(), Commands::Config(_)));
    }

    #[test]
    fn parse_reseal() {
        let cli = Cli::try_parse_from(["ivaldi", "reseal"]).unwrap();
        match cli.command.unwrap() {
            Commands::Reseal(args) => assert!(args.get_message().is_none()),
            _ => panic!("expected Reseal"),
        }

        let cli = Cli::try_parse_from(["ivaldi", "reseal", "new msg"]).unwrap();
        match cli.command.unwrap() {
            Commands::Reseal(args) => assert_eq!(args.get_message(), Some("new msg")),
            _ => panic!("expected Reseal"),
        }

        let cli = Cli::try_parse_from(["ivaldi", "reseal", "-m", "flag msg"]).unwrap();
        match cli.command.unwrap() {
            Commands::Reseal(args) => assert_eq!(args.get_message(), Some("flag msg")),
            _ => panic!("expected Reseal"),
        }
    }

    #[test]
    fn parse_undo_and_pluck() {
        let cli = Cli::try_parse_from(["ivaldi", "undo", "swift-eagle"]).unwrap();
        match cli.command.unwrap() {
            Commands::Undo(args) => {
                assert_eq!(args.seal, "swift-eagle");
                assert!(args.m.is_none());
            }
            _ => panic!("expected Undo"),
        }

        let cli = Cli::try_parse_from(["ivaldi", "pluck", "abc123", "-m", "msg"]).unwrap();
        match cli.command.unwrap() {
            Commands::Pluck(args) => {
                assert_eq!(args.seal, "abc123");
                assert_eq!(args.m.as_deref(), Some("msg"));
            }
            _ => panic!("expected Pluck"),
        }

        // git muscle memory works.
        let cli = Cli::try_parse_from(["ivaldi", "cherry-pick", "abc123"]).unwrap();
        assert!(matches!(cli.command.unwrap(), Commands::Pluck(_)));
    }

    #[test]
    fn parse_rewind_and_discard() {
        let cli = Cli::try_parse_from(["ivaldi", "rewind", "swift-eagle"]).unwrap();
        match cli.command.unwrap() {
            Commands::Rewind(args) => {
                assert_eq!(args.seal, "swift-eagle");
                assert!(!args.discard);
            }
            _ => panic!("expected Rewind"),
        }

        let cli = Cli::try_parse_from(["ivaldi", "rewind", "abc123", "--discard"]).unwrap();
        match cli.command.unwrap() {
            Commands::Rewind(args) => assert!(args.discard),
            _ => panic!("expected Rewind"),
        }

        // Rewind requires a seal.
        assert!(Cli::try_parse_from(["ivaldi", "rewind"]).is_err());

        // Discard only ungathers: files, or nothing for everything.
        let cli = Cli::try_parse_from(["ivaldi", "discard", "file.txt"]).unwrap();
        match cli.command.unwrap() {
            Commands::Discard(args) => assert_eq!(args.files, vec!["file.txt"]),
            _ => panic!("expected Discard"),
        }
        let cli = Cli::try_parse_from(["ivaldi", "discard"]).unwrap();
        match cli.command.unwrap() {
            Commands::Discard(args) => assert!(args.files.is_empty()),
            _ => panic!("expected Discard"),
        }

        // Reverse demands the explicit --all confirmation.
        let cli = Cli::try_parse_from(["ivaldi", "reverse", "--all"]).unwrap();
        match cli.command.unwrap() {
            Commands::Reverse(args) => assert!(args.all),
            _ => panic!("expected Reverse"),
        }
        assert!(Cli::try_parse_from(["ivaldi", "reverse"]).is_err());
    }

    #[test]
    fn parse_timeline_create() {
        let cli = Cli::try_parse_from(["ivaldi", "timeline", "create", "feature"]).unwrap();
        match cli.command.unwrap() {
            Commands::Timeline(args) => match args.command {
                TimelineCommands::Create(c) => {
                    assert_eq!(c.name, "feature");
                    assert!(c.from.is_none());
                }
                _ => panic!("expected Create"),
            },
            _ => panic!("expected Timeline"),
        }
    }

    #[test]
    fn parse_timeline_create_from() {
        let cli = Cli::try_parse_from(["ivaldi", "tl", "create", "hotfix", "main"]).unwrap();
        match cli.command.unwrap() {
            Commands::Timeline(args) => match args.command {
                TimelineCommands::Create(c) => {
                    assert_eq!(c.name, "hotfix");
                    assert_eq!(c.from.as_deref(), Some("main"));
                }
                _ => panic!("expected Create"),
            },
            _ => panic!("expected Timeline"),
        }
    }

    #[test]
    fn parse_timeline_cr_alias() {
        // "cr" should work as alias for "create"
        let cli = Cli::try_parse_from(["ivaldi", "tl", "cr", "experiment"]).unwrap();
        match cli.command.unwrap() {
            Commands::Timeline(args) => match args.command {
                TimelineCommands::Create(c) => {
                    assert_eq!(c.name, "experiment");
                }
                _ => panic!("expected Create via cr alias"),
            },
            _ => panic!("expected Timeline"),
        }
    }

    #[test]
    fn parse_timeline_sw_alias() {
        let cli = Cli::try_parse_from(["ivaldi", "tl", "sw", "main"]).unwrap();
        match cli.command.unwrap() {
            Commands::Timeline(args) => match args.command {
                TimelineCommands::Switch(s) => assert_eq!(s.name, "main"),
                _ => panic!("expected Switch"),
            },
            _ => panic!("expected Timeline"),
        }
    }

    #[test]
    fn parse_wai_alias() {
        let cli = Cli::try_parse_from(["ivaldi", "wai"]).unwrap();
        assert!(matches!(cli.command.unwrap(), Commands::Whereami));
    }

    #[test]
    fn parse_timeline_rename_one_arg() {
        let cli = Cli::try_parse_from(["ivaldi", "tl", "rename", "new-name"]).unwrap();
        match cli.command.unwrap() {
            Commands::Timeline(args) => match args.command {
                TimelineCommands::Rename(r) => assert_eq!(r.names, vec!["new-name"]),
                _ => panic!("expected Rename"),
            },
            _ => panic!("expected Timeline"),
        }
    }

    #[test]
    fn parse_timeline_rename_two_args() {
        let cli = Cli::try_parse_from(["ivaldi", "tl", "rename", "master", "main"]).unwrap();
        match cli.command.unwrap() {
            Commands::Timeline(args) => match args.command {
                TimelineCommands::Rename(r) => assert_eq!(r.names, vec!["master", "main"]),
                _ => panic!("expected Rename"),
            },
            _ => panic!("expected Timeline"),
        }
    }

    #[test]
    fn parse_timeline_rename_with_to_connector() {
        let cli = Cli::try_parse_from(["ivaldi", "tl", "rename", "master", "to", "main"]).unwrap();
        match cli.command.unwrap() {
            Commands::Timeline(args) => match args.command {
                TimelineCommands::Rename(r) => {
                    assert_eq!(r.names, vec!["master", "to", "main"])
                }
                _ => panic!("expected Rename"),
            },
            _ => panic!("expected Timeline"),
        }
    }

    #[test]
    fn parse_timeline_rn_alias() {
        let cli = Cli::try_parse_from(["ivaldi", "tl", "rn", "new-name"]).unwrap();
        match cli.command.unwrap() {
            Commands::Timeline(args) => match args.command {
                TimelineCommands::Rename(r) => assert_eq!(r.names, vec!["new-name"]),
                _ => panic!("expected Rename via rn alias"),
            },
            _ => panic!("expected Timeline"),
        }
    }

    #[test]
    fn parse_global_flags() {
        let cli = Cli::try_parse_from(["ivaldi", "-vv", "-q", "status"]).unwrap();
        assert_eq!(cli.verbose, 2);
        assert!(cli.quiet);
    }

    #[test]
    fn parse_forge_init_alias() {
        let cli = Cli::try_parse_from(["ivaldi", "init"]).unwrap();
        assert!(matches!(cli.command.unwrap(), Commands::Forge));
    }

    #[test]
    fn parse_gather_add_alias() {
        let cli = Cli::try_parse_from(["ivaldi", "add", "file.txt"]).unwrap();
        match cli.command.unwrap() {
            Commands::Gather(args) => {
                assert_eq!(args.files, vec!["file.txt"]);
            }
            _ => panic!("expected Gather"),
        }
    }

    #[test]
    fn parse_diff_no_targets() {
        let cli = Cli::try_parse_from(["ivaldi", "diff"]).unwrap();
        match cli.command.unwrap() {
            Commands::Diff(args) => {
                assert!(args.targets.is_empty());
                assert!(!args.staged);
                assert!(!args.stat);
            }
            _ => panic!("expected Diff"),
        }
    }

    #[test]
    fn parse_diff_one_target() {
        let cli = Cli::try_parse_from(["ivaldi", "diff", "crimson-forge"]).unwrap();
        match cli.command.unwrap() {
            Commands::Diff(args) => {
                assert_eq!(args.targets, vec!["crimson-forge"]);
            }
            _ => panic!("expected Diff"),
        }
    }

    #[test]
    fn parse_diff_two_targets() {
        let cli = Cli::try_parse_from(["ivaldi", "diff", "main", "feature"]).unwrap();
        match cli.command.unwrap() {
            Commands::Diff(args) => {
                assert_eq!(args.targets, vec!["main", "feature"]);
            }
            _ => panic!("expected Diff"),
        }
    }

    #[test]
    fn parse_diff_staged_flag() {
        let cli = Cli::try_parse_from(["ivaldi", "diff", "--staged"]).unwrap();
        match cli.command.unwrap() {
            Commands::Diff(args) => {
                assert!(args.staged);
                assert!(args.targets.is_empty());
            }
            _ => panic!("expected Diff"),
        }
    }

    #[test]
    fn parse_diff_stat_flag() {
        let cli = Cli::try_parse_from(["ivaldi", "diff", "--stat"]).unwrap();
        match cli.command.unwrap() {
            Commands::Diff(args) => {
                assert!(args.stat);
            }
            _ => panic!("expected Diff"),
        }
    }

    #[test]
    fn parse_tui_command() {
        let cli = Cli::try_parse_from(["ivaldi", "tui"]).unwrap();
        assert!(matches!(cli.command.unwrap(), Commands::Tui));
    }

    // ---- Review command parsing ----

    #[test]
    fn parse_review_create() {
        let cli = Cli::try_parse_from([
            "ivaldi",
            "review",
            "create",
            "--source",
            "feature",
            "--target",
            "main",
            "--title",
            "Add login",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Commands::Review(args) => match args.command {
                ReviewCommands::Create(c) => {
                    assert_eq!(c.source, "feature");
                    assert_eq!(c.target, "main");
                    assert_eq!(c.title, "Add login");
                    assert_eq!(c.strategy, "auto");
                }
                _ => panic!("expected Create"),
            },
            _ => panic!("expected Review"),
        }
    }

    #[test]
    fn parse_review_rv_alias() {
        let cli = Cli::try_parse_from([
            "ivaldi", "rv", "create", "--source", "feature", "--title", "Test",
        ])
        .unwrap();
        assert!(matches!(cli.command.unwrap(), Commands::Review(_)));
    }

    #[test]
    fn parse_review_list() {
        let cli = Cli::try_parse_from(["ivaldi", "review", "list"]).unwrap();
        match cli.command.unwrap() {
            Commands::Review(args) => match args.command {
                ReviewCommands::List(l) => {
                    assert!(l.status.is_none());
                    assert!(!l.all);
                }
                _ => panic!("expected List"),
            },
            _ => panic!("expected Review"),
        }
    }

    #[test]
    fn parse_review_list_with_status() {
        let cli = Cli::try_parse_from(["ivaldi", "review", "list", "--status", "open"]).unwrap();
        match cli.command.unwrap() {
            Commands::Review(args) => match args.command {
                ReviewCommands::List(l) => {
                    assert_eq!(l.status.as_deref(), Some("open"));
                }
                _ => panic!("expected List"),
            },
            _ => panic!("expected Review"),
        }
    }

    #[test]
    fn parse_review_show() {
        let cli = Cli::try_parse_from(["ivaldi", "review", "show", "42"]).unwrap();
        match cli.command.unwrap() {
            Commands::Review(args) => match args.command {
                ReviewCommands::Show(s) => assert_eq!(s.id, 42),
                _ => panic!("expected Show"),
            },
            _ => panic!("expected Review"),
        }
    }

    #[test]
    fn parse_review_diff() {
        let cli = Cli::try_parse_from(["ivaldi", "review", "diff", "1", "--stat"]).unwrap();
        match cli.command.unwrap() {
            Commands::Review(args) => match args.command {
                ReviewCommands::Diff(d) => {
                    assert_eq!(d.id, 1);
                    assert!(d.stat);
                }
                _ => panic!("expected Diff"),
            },
            _ => panic!("expected Review"),
        }
    }

    #[test]
    fn parse_review_comment() {
        let cli = Cli::try_parse_from([
            "ivaldi",
            "review",
            "comment",
            "1",
            "--file",
            "src/main.rs",
            "--line",
            "42",
            "--body",
            "Fix this",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Commands::Review(args) => match args.command {
                ReviewCommands::Comment(c) => {
                    assert_eq!(c.id, 1);
                    assert_eq!(c.file, "src/main.rs");
                    assert_eq!(c.line, Some(42));
                    assert_eq!(c.body, "Fix this");
                }
                _ => panic!("expected Comment"),
            },
            _ => panic!("expected Review"),
        }
    }

    #[test]
    fn parse_review_approve() {
        let cli = Cli::try_parse_from(["ivaldi", "review", "approve", "3"]).unwrap();
        match cli.command.unwrap() {
            Commands::Review(args) => match args.command {
                ReviewCommands::Approve(a) => assert_eq!(a.id, 3),
                _ => panic!("expected Approve"),
            },
            _ => panic!("expected Review"),
        }
    }

    #[test]
    fn parse_review_request_changes() {
        let cli = Cli::try_parse_from([
            "ivaldi",
            "review",
            "request-changes",
            "1",
            "--body",
            "Needs work",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Commands::Review(args) => match args.command {
                ReviewCommands::RequestChanges(rc) => {
                    assert_eq!(rc.id, 1);
                    assert_eq!(rc.body, "Needs work");
                }
                _ => panic!("expected RequestChanges"),
            },
            _ => panic!("expected Review"),
        }
    }

    #[test]
    fn parse_review_merge() {
        let cli = Cli::try_parse_from(["ivaldi", "review", "merge", "5"]).unwrap();
        match cli.command.unwrap() {
            Commands::Review(args) => match args.command {
                ReviewCommands::Merge(m) => {
                    assert_eq!(m.id, 5);
                    assert!(m.strategy.is_none());
                }
                _ => panic!("expected Merge"),
            },
            _ => panic!("expected Review"),
        }
    }

    #[test]
    fn parse_review_merge_with_strategy() {
        let cli = Cli::try_parse_from(["ivaldi", "review", "merge", "5", "--strategy", "theirs"])
            .unwrap();
        match cli.command.unwrap() {
            Commands::Review(args) => match args.command {
                ReviewCommands::Merge(m) => {
                    assert_eq!(m.strategy.as_deref(), Some("theirs"));
                }
                _ => panic!("expected Merge"),
            },
            _ => panic!("expected Review"),
        }
    }

    #[test]
    fn parse_review_close() {
        let cli = Cli::try_parse_from(["ivaldi", "review", "close", "2"]).unwrap();
        match cli.command.unwrap() {
            Commands::Review(args) => match args.command {
                ReviewCommands::Close(c) => assert_eq!(c.id, 2),
                _ => panic!("expected Close"),
            },
            _ => panic!("expected Review"),
        }
    }

    #[test]
    fn parse_status_json() {
        let cli = Cli::try_parse_from(["ivaldi", "status", "--json"]).unwrap();
        match cli.command.unwrap() {
            Commands::Status(args) => assert!(args.json),
            _ => panic!("expected Status"),
        }

        // Bare status still parses with json off.
        let cli = Cli::try_parse_from(["ivaldi", "status"]).unwrap();
        match cli.command.unwrap() {
            Commands::Status(args) => assert!(!args.json),
            _ => panic!("expected Status"),
        }
    }

    #[test]
    fn parse_log_format_json() {
        let cli = Cli::try_parse_from(["ivaldi", "log", "--format", "json"]).unwrap();
        match cli.command.unwrap() {
            Commands::Log(args) => {
                assert_eq!(args.format, Some(LogFormat::Json));
                assert!(!args.oneline);
            }
            _ => panic!("expected Log"),
        }
    }

    #[test]
    fn parse_log_oneline_still_works() {
        let cli = Cli::try_parse_from(["ivaldi", "log", "--oneline"]).unwrap();
        match cli.command.unwrap() {
            Commands::Log(args) => {
                assert!(args.oneline);
                assert!(args.format.is_none());
            }
            _ => panic!("expected Log"),
        }
    }

    #[test]
    fn parse_log_oneline_conflicts_with_format() {
        assert!(Cli::try_parse_from(["ivaldi", "log", "--oneline", "--format", "json"]).is_err());
    }

    #[test]
    fn parse_timeline_list_json() {
        let cli = Cli::try_parse_from(["ivaldi", "timeline", "list", "--json"]).unwrap();
        match cli.command.unwrap() {
            Commands::Timeline(args) => match args.command {
                TimelineCommands::List(l) => assert!(l.json),
                _ => panic!("expected List"),
            },
            _ => panic!("expected Timeline"),
        }
    }

    #[test]
    fn parse_completions_fish() {
        let cli = Cli::try_parse_from(["ivaldi", "completions", "fish"]).unwrap();
        match cli.command.unwrap() {
            Commands::Completions(args) => {
                assert_eq!(args.shell, CompletionShell::Fish);
            }
            _ => panic!("expected Completions"),
        }
    }

    #[test]
    fn parse_completions_raven() {
        let cli = Cli::try_parse_from(["ivaldi", "completions", "raven"]).unwrap();
        match cli.command.unwrap() {
            Commands::Completions(args) => {
                assert_eq!(args.shell, CompletionShell::Raven);
                assert!(args.shell.clap_shell().is_none());
            }
            _ => panic!("expected Completions"),
        }
    }

    #[test]
    fn parse_completions_powershell_keeps_one_word_value() {
        // clap_complete spells it `powershell`; the derived ValueEnum must too.
        let cli = Cli::try_parse_from(["ivaldi", "completions", "powershell"]).unwrap();
        match cli.command.unwrap() {
            Commands::Completions(args) => assert_eq!(args.shell, CompletionShell::PowerShell),
            _ => panic!("expected Completions"),
        }
    }

    #[test]
    fn completions_fish_generates_nonempty_output() {
        let mut buf = Vec::new();
        clap_complete::generate(
            clap_complete::Shell::Fish,
            &mut <Cli as clap::CommandFactory>::command(),
            "ivaldi",
            &mut buf,
        );
        assert!(!buf.is_empty());
        let script = String::from_utf8(buf).unwrap();
        assert!(script.contains("ivaldi"));
    }

    #[test]
    fn parse_review_reopen() {
        let cli = Cli::try_parse_from(["ivaldi", "review", "reopen", "2"]).unwrap();
        match cli.command.unwrap() {
            Commands::Review(args) => match args.command {
                ReviewCommands::Reopen(r) => assert_eq!(r.id, 2),
                _ => panic!("expected Reopen"),
            },
            _ => panic!("expected Review"),
        }
    }

    // ---- New short command aliases ----

    #[test]
    fn parse_top_level_short_aliases() {
        // A representative sample of the new 2-char aliases.
        assert!(matches!(
            Cli::try_parse_from(["ivaldi", "se"])
                .unwrap()
                .command
                .unwrap(),
            Commands::Seal(_)
        ));
        assert!(matches!(
            Cli::try_parse_from(["ivaldi", "st"])
                .unwrap()
                .command
                .unwrap(),
            Commands::Status(_)
        ));
        assert!(matches!(
            Cli::try_parse_from(["ivaldi", "up"])
                .unwrap()
                .command
                .unwrap(),
            Commands::Upload(_)
        ));
        assert!(matches!(
            Cli::try_parse_from(["ivaldi", "dl", "owner/repo"])
                .unwrap()
                .command
                .unwrap(),
            Commands::Download(_)
        ));
        assert!(matches!(
            Cli::try_parse_from(["ivaldi", "sy"])
                .unwrap()
                .command
                .unwrap(),
            Commands::Sync(_)
        ));
        assert!(matches!(
            Cli::try_parse_from(["ivaldi", "lg"])
                .unwrap()
                .command
                .unwrap(),
            Commands::Log(_)
        ));
    }

    #[test]
    fn short_aliases_do_not_collide_at_top_level() {
        // `up` is Upload at the top level; the same string is a Butterfly
        // subcommand alias, so they must not interfere across levels.
        assert!(matches!(
            Cli::try_parse_from(["ivaldi", "up"])
                .unwrap()
                .command
                .unwrap(),
            Commands::Upload(_)
        ));
        match Cli::try_parse_from(["ivaldi", "tl", "bf", "up"])
            .unwrap()
            .command
            .unwrap()
        {
            Commands::Timeline(t) => match t.command {
                TimelineCommands::Butterfly(b) => {
                    assert!(matches!(b.command, ButterflyCommands::Up))
                }
                _ => panic!("expected Butterfly"),
            },
            _ => panic!("expected Timeline"),
        }
    }

    #[test]
    fn parse_subcommand_short_aliases() {
        // review merge -> `mg`
        match Cli::try_parse_from(["ivaldi", "rv", "mg", "5"])
            .unwrap()
            .command
            .unwrap()
        {
            Commands::Review(args) => assert!(matches!(args.command, ReviewCommands::Merge(_))),
            _ => panic!("expected Review"),
        }
        // auth status -> `au st`
        match Cli::try_parse_from(["ivaldi", "au", "st"])
            .unwrap()
            .command
            .unwrap()
        {
            Commands::Auth(args) => assert!(matches!(args.command, AuthCommands::Status)),
            _ => panic!("expected Auth"),
        }
    }

    #[test]
    fn parse_auth_login_with_token_and_force() {
        match Cli::try_parse_from(["ivaldi", "auth", "login", "--with-token"])
            .unwrap()
            .command
            .unwrap()
        {
            Commands::Auth(args) => match args.command {
                AuthCommands::Login(l) => {
                    assert!(l.with_token);
                    assert!(!l.force);
                }
                _ => panic!("expected Login"),
            },
            _ => panic!("expected Auth"),
        }

        match Cli::try_parse_from(["ivaldi", "auth", "login", "--force"])
            .unwrap()
            .command
            .unwrap()
        {
            Commands::Auth(args) => match args.command {
                AuthCommands::Login(l) => {
                    assert!(l.force);
                    assert!(!l.with_token);
                }
                _ => panic!("expected Login"),
            },
            _ => panic!("expected Auth"),
        }
    }
}
