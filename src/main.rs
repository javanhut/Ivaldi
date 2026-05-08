use clap::Parser;
use ivaldi::cli::{Cli, run_command};

fn main() {
    // Don't panic if the user pipes our output through `head`, `grep -q`,
    // etc. and the reader closes early. Replace the default panic with a
    // silent exit-141 for any BrokenPipe payload coming out of stdout/stderr.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let msg = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("");
        if msg.contains("Broken pipe") {
            std::process::exit(141);
        }
        default_hook(info);
    }));

    let cli = Cli::parse();

    ivaldi::color::init();
    ivaldi::logging::init(cli.verbose, cli.quiet);

    run_command(cli);
}
