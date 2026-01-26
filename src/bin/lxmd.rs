use clap::Parser;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    config: Option<String>,
    #[arg(long)]
    rnsconfig: Option<String>,
    #[arg(short = 'p', long)]
    propagation_node: bool,
    #[arg(short = 'i', long)]
    on_inbound: Option<String>,
    #[arg(short = 'v', long)]
    verbose: bool,
    #[arg(short = 'q', long)]
    quiet: bool,
    #[arg(short = 's', long)]
    service: bool,
    #[arg(long)]
    exampleconfig: bool,
}

fn main() {
    let _ = Args::parse();
}
