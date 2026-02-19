use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "lxmf", about = "LXMF operator CLI", version)]
struct Cli {
    #[arg(long, default_value = "default")]
    profile: String,

    #[arg(long)]
    rpc: Option<String>,

    #[arg(long)]
    json: bool,

    #[arg(long)]
    quiet: bool,

    #[arg(long)]
    command: Option<String>,
}

fn main() {
    let _cli = Cli::parse();
}
