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
        // No subcommand is the canonical product entry point.
        None => {
            #[cfg(feature = "tui")]
            {
                let runtime =
                    styrene_tui_app::RuntimeContext::resolve(styrene_tui_app::RuntimeOverrides {
                        ghost: cli.ghost,
                        portable: cli.portable,
                    })
                    .map_err(anyhow::Error::msg)?;
                styrene_tui_app::run(styrene_tui_app::TuiOptions {
                    paths: runtime.paths,
                    runtime_profile: runtime.profile,
                })
                .await
            }
            #[cfg(not(feature = "tui"))]
            {
                // Minimal builds deliberately omit the interactive application.
                let _ = Cli::parse_from(["styrene", "--help"]);
                Ok(())
            }
        }

        #[cfg(feature = "tui")]
        Some(Command::Doctor { root }) => {
            let paths = styrene_tui_app::StyrenePaths::new(
                root.join("config"),
                root.join("data"),
                root.join("run/styrene.sock"),
                root.join("home"),
            );
            styrene_tui_app::run_clean_room_check(&paths)?;
            println!("styrene doctor: ok ({})", root.display());
            Ok(())
        }

        #[cfg(feature = "daemon")]
        Some(Command::Daemon { rpc: _, db, config, identity, ephemeral }) => {
            styrened::daemon::run(styrened::daemon::DaemonConfig2 {
                db,
                config,
                identity,
                socket: socket.map(std::path::PathBuf::from),
                ephemeral,
            })
            .await
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
        Some(Command::Tunnel { ref action }) => match action {
            cli::TunnelAction::List => commands::tunnel_list(socket).await,
            cli::TunnelAction::Status { ref peer } => commands::tunnel_status(socket, peer).await,
            cli::TunnelAction::Establish { ref peer } => {
                commands::tunnel_establish(socket, peer).await
            }
            cli::TunnelAction::Offer { ref peer } => commands::tunnel_offer(socket, peer).await,
            cli::TunnelAction::Teardown { ref peer } => {
                commands::tunnel_teardown(socket, peer).await
            }
        },

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
            cli::FleetAction::Apply { ref node, ref profile, no_verify, timeout } => {
                commands::fleet_apply(socket, node, profile, !no_verify, *timeout).await
            }
            cli::FleetAction::Grant { ref node, ref role, ref label, ref grants } => {
                commands::fleet_grant(socket, node, role, label.as_deref(), grants).await
            }
            cli::FleetAction::Revoke { ref node } => commands::fleet_revoke(socket, node).await,
        },
    }
}
