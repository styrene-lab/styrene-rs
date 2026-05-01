//! styrene-i2p — Local HTTP proxy for .i2p eepsites over the Styrene mesh.
//!
//! Captures browser HTTP requests to `.i2p` domains, serializes them as
//! I2pProxyRequest messages, sends them over RNS to the hub's I2pProxyService,
//! reassembles chunked responses, and returns them to the browser.
//!
//! Usage:
//!   styrene-i2p                          # Start proxy on 127.0.0.1:4480
//!   styrene-i2p --bind 0.0.0.0:8080     # Custom bind address
//!   styrene-i2p --hub <identity_hash>   # Specify hub identity
//!   styrene-i2p --install-service        # Install as launchd/systemd service

mod mesh_client;
mod proxy;

use clap::Parser;

#[derive(Parser)]
#[command(name = "styrene-i2p", about = "I2P eepsite proxy over Styrene mesh")]
struct Cli {
    /// Local bind address for the HTTP proxy
    #[arg(long, default_value = "127.0.0.1:4480")]
    bind: String,

    /// Hub delivery destination hash (hex, required for mesh mode)
    #[arg(long)]
    hub: Option<String>,

    /// Hub TCP address for mesh transport (e.g., 192.168.0.10:4242)
    #[arg(long, env = "STYRENE_HUB_ADDR")]
    hub_addr: Option<String>,

    /// i2pd HTTP proxy address.
    /// Local i2pd: http://127.0.0.1:4444
    /// Hub's i2pd (via port-forward or direct): http://<hub-ip>:4444
    /// Omit to use hub discovery over mesh (not yet implemented).
    #[arg(long, env = "STYRENE_I2PD_ADDR")]
    i2pd: Option<String>,

    /// Install as a system service (launchd on macOS, systemd on Linux)
    #[arg(long)]
    install_service: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.install_service {
        install_service(&cli.bind, cli.i2pd.as_deref(), cli.hub_addr.as_deref(), cli.hub.as_deref())?;
        return Ok(());
    }

    // Mesh mode: --hub-addr + --hub (RNS transport to hub's I2pProxyService)
    if let (Some(ref hub_addr), Some(ref hub_hash)) = (&cli.hub_addr, &cli.hub) {
        eprintln!("[styrene-i2p] mesh mode — hub at {hub_addr}, hash {hub_hash}");
        let client = mesh_client::MeshClient::new(hub_addr, hub_hash, None).await?;
        let client = std::sync::Arc::new(client);
        proxy::run_mesh(&cli.bind, client).await?;
        return Ok(());
    }

    // Direct mode: --i2pd or auto-discover local i2pd
    let i2pd_addr = match cli.i2pd {
        Some(ref addr) => addr.clone(),
        None => {
            let defaults = [
                "http://127.0.0.1:4444",
                "http://i2pd.styrene-forge.svc:4444",
            ];
            let mut found = None;
            for addr in defaults {
                if let Ok(client) = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(2))
                    .build()
                {
                    if client.get(addr).send().await.is_ok() {
                        found = Some(addr.to_string());
                        break;
                    }
                }
            }
            found.unwrap_or_else(|| {
                eprintln!("[styrene-i2p] no i2pd found at default addresses");
                eprintln!("[styrene-i2p] options:");
                eprintln!("  --i2pd http://127.0.0.1:4444                 # local i2pd (direct mode)");
                eprintln!("  --hub-addr 192.168.0.10:4242 --hub <hash>    # mesh mode via hub");
                std::process::exit(1);
            })
        }
    };

    eprintln!("[styrene-i2p] direct mode — proxy on {} → i2pd at {}", cli.bind, i2pd_addr);
    proxy::run_direct(&cli.bind, &i2pd_addr).await?;

    Ok(())
}

fn install_service(bind: &str, i2pd: Option<&str>, hub_addr: Option<&str>, hub: Option<&str>) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let exe_path = exe.display();

    // Build args based on mode
    let mut args = vec![format!("--bind {bind}")];
    if let (Some(ha), Some(h)) = (hub_addr, hub) {
        args.push(format!("--hub-addr {ha}"));
        args.push(format!("--hub {h}"));
    } else {
        let i2pd_addr = i2pd.unwrap_or("http://127.0.0.1:4444");
        args.push(format!("--i2pd {i2pd_addr}"));
    }
    let args_str = args.join(" ");

    #[cfg(target_os = "macos")]
    {
        let mut arg_xml = format!(
            "        <string>{exe_path}</string>\n"
        );
        for arg in args_str.split_whitespace() {
            arg_xml.push_str(&format!("        <string>{arg}</string>\n"));
        }

        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>io.styrene.i2p-proxy</string>
    <key>ProgramArguments</key>
    <array>
{arg_xml}    </array>
    <key>KeepAlive</key>
    <true/>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/styrene-i2p.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/styrene-i2p.log</string>
</dict>
</plist>"#
        );

        let plist_path = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
            .join("Library/LaunchAgents/io.styrene.i2p-proxy.plist");

        std::fs::write(&plist_path, plist)?;
        eprintln!("Installed launchd plist at {}", plist_path.display());
        eprintln!("Load with: launchctl load {}", plist_path.display());
    }

    #[cfg(target_os = "linux")]
    {
        let unit = format!(
            r#"[Unit]
Description=Styrene I2P Proxy
After=network.target

[Service]
ExecStart={exe_path} {args_str}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#
        );

        let unit_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("cannot determine config directory"))?
            .join("systemd/user");
        std::fs::create_dir_all(&unit_dir)?;
        let unit_path = unit_dir.join("styrene-i2p-proxy.service");
        std::fs::write(&unit_path, unit)?;
        eprintln!("Installed systemd unit at {}", unit_path.display());
        eprintln!("Enable with: systemctl --user enable --now styrene-i2p-proxy");
    }

    Ok(())
}
