use clap::Parser;

/// Styrene mesh daemon (Rust implementation).
///
/// This is a skeleton — active development begins after the interop gate
/// (Phase 3 of the migration plan).
#[derive(Parser, Debug)]
#[command(name = "styrened-rs", version, about)]
struct Cli {
    /// Configuration file path
    #[arg(short, long, default_value = "/etc/styrene/config.toml")]
    config: String,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if cli.verbose {
        eprintln!("styrened-rs: config={}", cli.config);
    }

    eprintln!("styrened-rs is a skeleton — active development after interop gate (Phase 3)");
    eprintln!("See: https://github.com/styrene-lab/styrene-rs");
}
