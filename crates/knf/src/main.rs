use clap::Parser;

fn main() {
    // clap handles --help/--version and exits 2 on usage errors.
    let cli = knf::cli::Cli::parse();

    if let Err(err) = knf::run(cli) {
        // Some errors (the null-in-TOML report, the ambiguous-paths report) are
        // deliberately multi-line and carry their own `help:` line.
        eprintln!("error: {err}");
        for cause in err.chain().skip(1) {
            eprintln!("  caused by: {cause}");
        }
        std::process::exit(1);
    }
}
