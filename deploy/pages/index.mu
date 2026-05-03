`F58a`cStyrene Community Hub`f
`crns.styrene.io`f

-

`F999`cMesh comms for real hardware`f

`F888Encrypted messaging, cryptographic identity, device provisioning
over Reticulum. TCP, wireless mesh, LoRa radio.
Same protocol stack, whatever the transport.`f

-

>How It Works

>>Identity
Keys generated on-device. Ephemeral by default,
pinned to YubiKey if you want persistence.
No accounts, no registration, no server
deciding who you are.

>>Messaging
LXMF -- encrypted, store-and-forward. Messages
propagate through the mesh until they arrive.
Works with Sideband and NomadNet on the same wire.

>>Provisioning
Write a NixOS image. Plug it into hardware.
It joins the mesh on first boot.

-

>Getting Started

>>Install styrene-rs

    # Install via Homebrew
    brew tap styrene-lab/tap
    brew install styrene

    # Or download from GitHub Releases
    # https://github.com/styrene-lab/styrene-rs/releases

>>Connect to the Community Hub

    # Create config directory
    mkdir -p ~/.config/styrene

    # Add hub connection
    cat > ~/.config/styrene/config.toml << 'EOF'
    [[interfaces]]
    type = "tcp_client"
    enabled = true
    host = "rns.styrene.io"
    port = 4242
    name = "styrene-community-hub"
    EOF

>>Launch

    # Start the daemon
    styrened

    # Launch the desktop app
    styrene-dx

    # Or use the terminal UI
    styrene-tui

-

>Components

`F5af>>styrened`f
The mesh daemon. Runs in background, manages transport
interfaces, routes messages, serves pages.

`F5af>>styrene-dx`f
Desktop app (Dioxus). Network graph, conversations,
page browser. Connects to styrened via IPC or boots
one in-process.

`F5af>>styrene-tui`f
Terminal UI (Ratatui). Full operator interface for
mesh status, messaging, fleet management.

`F5af>>styrene-rs`f
Rust implementation of the RNS/LXMF protocol stack.
Wire-compatible with Python Reticulum.

-

>Transport Layers

`F9d9Internet`f -- TCP/IP (this hub)
`F9d9WireGuard Tunnel`f -- Full IP, negotiated via LXMF
`F9d9L2 Mesh`f -- BATMAN-adv / 802.11s
`F9d9LoRa Radio`f -- RNode hardware
`F9d9Automatic Promotion`f -- Upgrades transport without intervention

-

>Pages

`[Getting Started`:/page/getting-started.mu]
`[Architecture`:/page/architecture.mu]
`[Hardware Guide`:/page/hardware.mu]

-

>Community

GitHub -- https://github.com/styrene-lab/styrene-rs
Discord -- https://discord.gg/styrene
Reddit -- r/styrenelab

-

`F666`cStyrene Lab -- tools for sovereign infrastructure
(c) 2026 Black Meridian LLC`f
