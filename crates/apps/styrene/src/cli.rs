//! CLI argument definitions for the styrene binary.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "styrene", about = "Styrene mesh node — daemon, TUI, and CLI", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Path to the daemon IPC socket
    #[arg(long, global = true, env = "STYRENE_SOCKET")]
    pub socket: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run the mesh daemon (foreground)
    #[cfg(feature = "daemon")]
    Daemon {
        /// RPC listen address
        #[arg(long, default_value = "127.0.0.1:4243")]
        rpc: String,
        /// Database path
        #[arg(long)]
        db: Option<PathBuf>,
        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
        /// Identity file path
        #[arg(long)]
        identity: Option<PathBuf>,
        /// Use ephemeral identity (no persistence, for containers)
        #[arg(long)]
        ephemeral: bool,
    },

    /// Show daemon and mesh status
    #[cfg(feature = "cli")]
    Status,

    /// List known peers
    #[cfg(feature = "cli")]
    Peers {
        /// Search query
        query: Option<String>,
        /// Show only Styrene nodes
        #[arg(long)]
        styrene_only: bool,
    },

    /// Send a message to a peer
    #[cfg(feature = "cli")]
    Send {
        /// Destination peer hash or name
        destination: String,
        /// Message content
        content: String,
        /// Message title
        #[arg(long)]
        title: Option<String>,
    },

    /// Show messages with a peer
    #[cfg(feature = "cli")]
    Messages {
        /// Peer hash
        peer: String,
        /// Maximum messages to show
        #[arg(long, default_value = "20")]
        limit: u32,
    },

    /// Show daemon identity
    #[cfg(feature = "cli")]
    Identity,

    /// Trigger a mesh announce
    #[cfg(feature = "cli")]
    Announce,

    /// Show or modify daemon configuration
    #[cfg(feature = "cli")]
    Config,

    /// Fleet management — query and control remote nodes
    #[cfg(feature = "cli")]
    Fleet {
        #[command(subcommand)]
        action: FleetAction,
    },
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
pub enum FleetAction {
    /// Show status of all known nodes or a specific node
    Status {
        /// Specific node hash to query (all nodes if omitted)
        node: Option<String>,
        /// Timeout in seconds for remote queries
        #[arg(long, default_value = "10")]
        timeout: u64,
    },
    /// Execute a command on a remote node
    Exec {
        /// Target node destination hash
        node: String,
        /// Command to execute
        cmd: String,
        /// Command arguments
        args: Vec<String>,
        /// Timeout in seconds
        #[arg(long, default_value = "30")]
        timeout: u64,
    },
    /// Reboot a remote node
    Reboot {
        /// Target node destination hash
        node: String,
        /// Delay before reboot in seconds
        #[arg(long, default_value = "0")]
        delay: u64,
    },
}
