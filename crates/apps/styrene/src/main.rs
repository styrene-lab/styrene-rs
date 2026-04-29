mod cli;
#[cfg(feature = "cli")]
mod commands;
#[cfg(feature = "cli")]
#[allow(dead_code)] // API surface growing — not all methods wired to commands yet
mod ipc_client;

use clap::Parser;

use cli::{Cli, Command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let socket = cli.socket.as_deref();

    match cli.command {
        // No subcommand: launch TUI if available, otherwise print help
        None => {
            #[cfg(feature = "tui")]
            {
                // TODO: launch TUI with embedded daemon
                eprintln!("TUI not yet wired — run `styrene daemon` or `styrene status`");
                std::process::exit(1);
            }
            #[cfg(not(feature = "tui"))]
            {
                // Re-parse with --help to show usage
                let _ = Cli::parse_from(["styrene", "--help"]);
                Ok(())
            }
        }

        #[cfg(feature = "daemon")]
        Some(Command::Daemon { rpc: _, db: _, config: _, identity: _, ephemeral: _ }) => {
            // TODO: wire up styrened::run() with these args
            eprintln!("Daemon startup not yet wired — use `styrened` binary for now");
            std::process::exit(1);
        }

        #[cfg(feature = "cli")]
        Some(Command::Status) => commands::status(socket).await,

        #[cfg(feature = "cli")]
        Some(Command::Peers { ref query, styrene_only }) => {
            commands::peers(socket, query.as_deref(), styrene_only).await
        }

        #[cfg(feature = "cli")]
        Some(Command::Send { ref destination, ref content, ref title }) => {
            commands::send(socket, destination, content, title.as_deref()).await
        }

        #[cfg(feature = "cli")]
        Some(Command::Messages { ref peer, limit }) => {
            commands::messages(socket, peer, limit).await
        }

        #[cfg(feature = "cli")]
        Some(Command::Identity) => commands::identity(socket).await,

        #[cfg(feature = "cli")]
        Some(Command::Announce) => commands::announce(socket).await,

        #[cfg(feature = "cli")]
        Some(Command::Config) => commands::config(socket).await,

        #[cfg(feature = "cli")]
        Some(Command::Fleet { ref action }) => match action {
            cli::FleetAction::Status { ref node, timeout } => {
                commands::fleet_status(socket, node.as_deref(), *timeout).await
            }
            cli::FleetAction::Exec { ref node, ref cmd, ref args, timeout } => {
                commands::fleet_exec(socket, node, cmd, args, *timeout).await
            }
            cli::FleetAction::Reboot { ref node, delay } => {
                commands::fleet_reboot(socket, node, *delay).await
            }
        },
    }
}
