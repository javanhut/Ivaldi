use clap::Parser;
use ivaldi::cli::{Cli, run_command};

fn main() {
    let cli = Cli::parse();
    run_command(cli);
}
