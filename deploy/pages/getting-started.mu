`F58a`cGetting Started with Styrene`f

`[< Back to Index`:/page/index.mu]

-

>Prerequisites

A macOS or Linux machine. No Windows support.
No cloud accounts, no API keys, no registration.

-

>Install

>>Option 1: Homebrew (recommended)

    brew tap styrene-lab/tap
    brew install styrene

This installs three binaries:
  styrened    -- mesh daemon
  styrene-tui -- terminal UI
  styrene-dx  -- desktop app

>>Option 2: Download Binary

Download from GitHub Releases:
https://github.com/styrene-lab/styrene-rs/releases

Available archives:
  styrene-VERSION-aarch64-apple-darwin.tar.gz
  styrene-VERSION-x86_64-apple-darwin.tar.gz
  styrene-VERSION-x86_64-unknown-linux-gnu.tar.gz
  styrene-VERSION-aarch64-unknown-linux-gnu.tar.gz

>>Option 3: Build from Source

    git clone https://github.com/styrene-lab/styrene-rs
    cd styrene-rs
    cargo build --release -p styrened -p styrene-tui -p styrene-dx

-

>Configure

>>Connect to the Community Hub

    mkdir -p ~/.config/styrene

    cat > ~/.config/styrene/config.toml << 'CONF'
    [[interfaces]]
    type = "tcp_client"
    enabled = true
    host = "rns.styrene.io"
    port = 4242
    name = "styrene-community-hub"
    CONF

>>Set Your Display Name (optional)

    export LXMF_DISPLAY_NAME="YourCallsign"

-

>Launch

>>Desktop App
    styrene-dx

The desktop app boots an embedded daemon automatically.
It reads your config from ~/.config/styrene/config.toml.

>>Terminal UI
    styrene-tui

Full-featured operator interface in the terminal.

>>Daemon Only
    styrened

Run the daemon in background. The desktop app and TUI
connect to it via IPC socket.

-

>Verify

Once connected, you should see peers discovered from
the community hub. The network graph will show your
local node connected through the TCP transport to the
Styrene Community Hub, with other mesh peers radiating
outward.

>What Next

Send a message to another peer from the Conversations tab.
Browse pages from the Pages tab.
Explore the mesh topology in the Network tab.

-

`[< Back to Index`:/page/index.mu]
