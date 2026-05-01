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

mod proxy;

use clap::Parser;

#[derive(Parser)]
#[command(name = "styrene-i2p", about = "I2P eepsite proxy over Styrene mesh")]
struct Cli {
    /// Local bind address for the HTTP proxy
    #[arg(long, default_value = "127.0.0.1:4480")]
    bind: String,

    /// Hub identity hash (auto-discovers via mesh announces if omitted)
    #[arg(long)]
    hub: Option<String>,

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
        install_service(&cli.bind, cli.i2pd.as_deref())?;
        return Ok(());
    }

    let i2pd_addr = match cli.i2pd {
        Some(ref addr) => addr.clone(),
        None => {
            // Try common defaults
            let defaults = [
                "http://127.0.0.1:4444",     // Local i2pd
                "http://i2pd.styrene-forge.svc:4444", // K8s in-cluster
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
                eprintln!("[styrene-i2p] specify with --i2pd <addr> or STYRENE_I2PD_ADDR env");
                eprintln!("[styrene-i2p] examples:");
                eprintln!("  styrene-i2p --i2pd http://127.0.0.1:4444     # local i2pd");
                eprintln!("  styrene-i2p --i2pd http://192.168.0.10:4444  # hub i2pd");
                std::process::exit(1);
            })
        }
    };

    eprintln!("[styrene-i2p] proxy on {} → i2pd at {}", cli.bind, i2pd_addr);
    proxy::run_direct(&cli.bind, &i2pd_addr).await?;

    Ok(())
}

fn install_service(bind: &str, i2pd: Option<&str>) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let exe_path = exe.display();
    let i2pd_addr = i2pd.unwrap_or("http://127.0.0.1:4444");

    #[cfg(target_os = "macos")]
    {
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>io.styrene.i2p-proxy</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe_path}</string>
        <string>--bind</string>
        <string>{bind}</string>
        <string>--i2pd</string>
        <string>{i2pd_addr}</string>
    </array>
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
ExecStart={exe_path} --bind {bind} --i2pd {i2pd_addr}
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
