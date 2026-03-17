use clap::Parser;
use ivaldi::cli::{Cli, run_command};

fn main() {
    let cli = Cli::parse();

    // Initialize color and logging from flags
    ivaldi::color::init();
    ivaldi::logging::init(cli.verbose, cli.quiet);

    run_command(cli);
}
