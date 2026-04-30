//! CLI command implementations — one-shot IPC calls to the daemon.

use std::path::Path;

use console::style;
use toml;

use crate::ipc_client::DaemonClient;

/// Safely truncate a string to at most `n` characters (not bytes).
fn truncate(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

pub(crate) async fn status(socket: Option<&Path>) -> anyhow::Result<()> {
    let mut client = DaemonClient::connect(socket).await.map_err(anyhow::Error::msg)?;
    let status = client.status().await.map_err(anyhow::Error::msg)?;
    let identity = client.identity().await.map_err(anyhow::Error::msg)?;

    eprintln!();
    eprintln!("  {}", style("styrene status").cyan().bold());
    eprintln!();
    eprintln!("  identity   {}", identity.destination_hash);
    eprintln!("  name       {}", identity.display_name);
    eprintln!("  version    {}", status.daemon_version);
    eprintln!("  uptime     {}s", status.uptime);
    eprintln!(
        "  rns        {}",
        if status.rns_initialized {
            style("initialized").green()
        } else {
            style("not ready").red()
        }
    );
    eprintln!(
        "  transport  {}",
        if status.transport_enabled { style("active").green() } else { style("inactive").dim() }
    );
    eprintln!("  interfaces {}", status.interface_count);
    eprintln!("  peers      {}", status.device_count);
    eprintln!("  links      {}", status.active_links);
    eprintln!();

    Ok(())
}

pub(crate) async fn peers(
    socket: Option<&Path>,
    query: Option<&str>,
    styrene_only: bool,
) -> anyhow::Result<()> {
    let mut client = DaemonClient::connect(socket).await.map_err(anyhow::Error::msg)?;
    let devices = client.devices(styrene_only).await.map_err(anyhow::Error::msg)?;

    let filtered: Vec<_> = if let Some(q) = query {
        let q = q.to_lowercase();
        devices
            .iter()
            .filter(|d| {
                d.name.to_lowercase().contains(&q)
                    || d.destination_hash.contains(&q)
                    || d.identity_hash.contains(&q)
            })
            .collect()
    } else {
        devices.iter().collect()
    };

    eprintln!();
    eprintln!("  {} ({} peers)", style("styrene peers").cyan().bold(), filtered.len());
    eprintln!();

    for dev in &filtered {
        let name = if dev.name.is_empty() {
            style("(unnamed)").dim().to_string()
        } else {
            dev.name.clone()
        };
        let hash_short = truncate(&dev.destination_hash, 12);
        let styrene_marker = if dev.is_styrene_node {
            style("⬡").green().to_string()
        } else {
            style("○").dim().to_string()
        };
        eprintln!("  {styrene_marker} {hash_short}…  {name}");
    }
    eprintln!();

    Ok(())
}

pub(crate) async fn send(
    socket: Option<&Path>,
    destination: &str,
    content: &str,
    title: Option<&str>,
) -> anyhow::Result<()> {
    let mut client = DaemonClient::connect(socket).await.map_err(anyhow::Error::msg)?;
    let msg_id = client.send_chat(destination, content, title).await.map_err(anyhow::Error::msg)?;

    eprintln!(
        "  {} sent to {}  (id: {})",
        style("✓").green().bold(),
        truncate(destination, 12),
        truncate(&msg_id, 8)
    );

    Ok(())
}

pub(crate) async fn messages(socket: Option<&Path>, peer: &str, limit: u32) -> anyhow::Result<()> {
    let mut client = DaemonClient::connect(socket).await.map_err(anyhow::Error::msg)?;
    let msgs = client.messages(peer, limit).await.map_err(anyhow::Error::msg)?;

    let peer_short = truncate(peer, 12);
    eprintln!();
    eprintln!(
        "  {} ({} messages with {peer_short}…)",
        style("styrene messages").cyan().bold(),
        msgs.len()
    );
    eprintln!();

    for msg in &msgs {
        let direction = if msg.is_outgoing { style("→").cyan() } else { style("←").green() };
        let content_preview = if msg.content.chars().count() > 60 {
            format!("{}…", truncate(&msg.content, 60))
        } else {
            msg.content.clone()
        };
        eprintln!("  {direction} {content_preview}");
    }
    eprintln!();

    Ok(())
}

pub(crate) async fn identity(socket: Option<&Path>) -> anyhow::Result<()> {
    let mut client = DaemonClient::connect(socket).await.map_err(anyhow::Error::msg)?;
    let info = client.identity().await.map_err(anyhow::Error::msg)?;

    eprintln!();
    eprintln!("  {}", style("styrene identity").cyan().bold());
    eprintln!();
    eprintln!("  hash       {}", info.identity_hash);
    eprintln!("  dest       {}", info.destination_hash);
    eprintln!("  lxmf       {}", info.lxmf_destination_hash);
    eprintln!("  name       {}", info.display_name);
    if let Some(ref icon) = info.icon {
        eprintln!("  icon       {icon}");
    }
    eprintln!();

    Ok(())
}

pub(crate) async fn announce(socket: Option<&Path>) -> anyhow::Result<()> {
    let mut client = DaemonClient::connect(socket).await.map_err(anyhow::Error::msg)?;
    let ok = client.announce().await.map_err(anyhow::Error::msg)?;

    if ok {
        eprintln!("  {} announce sent", style("✓").green().bold());
    } else {
        eprintln!("  {} announce failed", style("✗").red().bold());
    }

    Ok(())
}

pub(crate) async fn config(socket: Option<&Path>) -> anyhow::Result<()> {
    let mut client = DaemonClient::connect(socket).await.map_err(anyhow::Error::msg)?;
    let cfg = client.config().await.map_err(anyhow::Error::msg)?;

    eprintln!();
    eprintln!("  {}", style("styrene config").cyan().bold());
    eprintln!();

    let mut keys: Vec<_> = cfg.keys().collect();
    keys.sort();
    for key in keys {
        let val = &cfg[key];
        eprintln!("  {key} = {val}");
    }
    eprintln!();

    Ok(())
}

// ── Tunnel operations ───────────────────────────────────────────────────────

pub(crate) async fn tunnel_list(socket: Option<&Path>) -> anyhow::Result<()> {
    let mut client = DaemonClient::connect(socket).await.map_err(anyhow::Error::msg)?;

    match client.list_tunnels().await {
        Ok(tunnels) => {
            eprintln!();
            eprintln!(
                "  {} ({} active)",
                style("styrene tunnel list").cyan().bold(),
                tunnels.len()
            );
            eprintln!();

            if tunnels.is_empty() {
                eprintln!("  {} no active tunnels", style("○").dim());
            }

            for t in &tunnels {
                let peer = t.get("peer_hash").and_then(|v| v.as_str()).unwrap_or("?");
                let state = t.get("state").and_then(|v| v.as_str()).unwrap_or("unknown");
                let endpoint = t.get("remote_endpoint").and_then(|v| v.as_str()).unwrap_or("");
                let marker = if state == "established" {
                    style("⬡").green().to_string()
                } else {
                    style("○").dim().to_string()
                };
                eprintln!("  {marker} {}…  {state}  {endpoint}", truncate(peer, 12));
            }
            eprintln!();
        }
        Err(e) => {
            // Fallback: show styrene peers instead
            let devices = client.devices(true).await.map_err(anyhow::Error::msg)?;
            eprintln!();
            eprintln!(
                "  {} ({} styrene peers)",
                style("styrene tunnel list").cyan().bold(),
                devices.len()
            );
            eprintln!("  {}", style(format!("tunnel query unavailable: {e}")).dim());
            eprintln!();
            for dev in &devices {
                let name =
                    if dev.name.is_empty() { "(unnamed)".to_string() } else { dev.name.clone() };
                let hash_short = truncate(&dev.destination_hash, 12);
                eprintln!("  {} {hash_short}…  {name}", style("○").dim());
            }
            eprintln!();
        }
    }

    Ok(())
}

pub(crate) async fn tunnel_status(socket: Option<&Path>, peer: &str) -> anyhow::Result<()> {
    let mut client = DaemonClient::connect(socket).await.map_err(anyhow::Error::msg)?;
    let devices = client.devices(true).await.map_err(anyhow::Error::msg)?;

    let peer_short = truncate(peer, 12);
    eprintln!();
    eprintln!("  {} ({peer_short}…)", style("styrene tunnel status").cyan().bold(),);
    eprintln!();

    let found = devices
        .iter()
        .find(|d| d.destination_hash.starts_with(peer) || d.identity_hash.starts_with(peer));

    if let Some(dev) = found {
        let name = if dev.name.is_empty() { "(unnamed)".to_string() } else { dev.name.clone() };
        eprintln!("  peer    {}", dev.destination_hash);
        eprintln!("  name    {name}");
        eprintln!("  status  {}", dev.status);
        eprintln!("  tunnel  {}", style("not yet available").dim());
    } else {
        eprintln!("  {} peer {peer_short}… not found", style("✗").red().bold());
    }
    eprintln!();

    Ok(())
}

pub(crate) fn tunnel_establish(peer: &str) {
    let peer_short = truncate(peer, 12);
    eprintln!();
    eprintln!("  {} tunnel establish ({peer_short}…)", style("⚠").yellow().bold(),);
    eprintln!();
    eprintln!(
        "  {}",
        style("not yet available — tunnel establishment is handled automatically via LXMF negotiation").dim()
    );
    eprintln!();
}

pub(crate) fn tunnel_teardown(peer: &str) {
    let peer_short = truncate(peer, 12);
    eprintln!();
    eprintln!("  {} tunnel teardown ({peer_short}…)", style("⚠").yellow().bold(),);
    eprintln!();
    eprintln!(
        "  {}",
        style("not yet available — tunnel establishment is handled automatically via LXMF negotiation").dim()
    );
    eprintln!();
}

// ── Fleet operations ────────────────────────────────────────────────────────

pub(crate) async fn fleet_status(
    socket: Option<&Path>,
    node: Option<&str>,
    timeout: u64,
) -> anyhow::Result<()> {
    let mut client = DaemonClient::connect(socket).await.map_err(anyhow::Error::msg)?;

    if let Some(dest) = node {
        let node_short = truncate(dest, 12);
        eprintln!();
        eprintln!("  {} (querying {node_short}…)", style("styrene fleet status").cyan().bold(),);

        let result = client.device_status(dest, timeout).await.map_err(anyhow::Error::msg)?;

        eprintln!();
        for (key, val) in &result {
            eprintln!("  {key}: {val}");
        }
        eprintln!();
    } else {
        let devices = client.devices(false).await.map_err(anyhow::Error::msg)?;

        eprintln!();
        eprintln!("  {} ({} nodes)", style("styrene fleet status").cyan().bold(), devices.len());
        eprintln!();

        for dev in &devices {
            let name = if dev.name.is_empty() { "(unnamed)".to_string() } else { dev.name.clone() };
            let hash_short = truncate(&dev.destination_hash, 12);
            let marker = if dev.is_styrene_node {
                style("⬡").green().to_string()
            } else {
                style("○").dim().to_string()
            };
            eprintln!("  {marker} {hash_short}…  {name}  {}", style(&dev.status).dim());
        }
        eprintln!();
    }

    Ok(())
}

pub(crate) async fn fleet_exec(
    socket: Option<&Path>,
    node: &str,
    cmd: &str,
    args: &[String],
    timeout: u64,
) -> anyhow::Result<()> {
    let mut client = DaemonClient::connect(socket).await.map_err(anyhow::Error::msg)?;

    let node_short = truncate(node, 12);
    eprintln!("  {} exec on {node_short}…: {cmd} {}", style("→").cyan(), args.join(" "));

    let result = client.exec(node, cmd, args, timeout).await.map_err(anyhow::Error::msg)?;

    let exit_code = result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
    let stdout = result.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    let stderr = result.get("stderr").and_then(|v| v.as_str()).unwrap_or("");

    if !stdout.is_empty() {
        print!("{stdout}");
        // Ensure newline before the exit code line
        if !stdout.ends_with('\n') {
            println!();
        }
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
        if !stderr.ends_with('\n') {
            eprintln!();
        }
    }

    if exit_code == 0 {
        eprintln!("  {} exit code {exit_code}", style("✓").green().bold());
    } else {
        eprintln!("  {} exit code {exit_code}", style("✗").red().bold());
    }

    Ok(())
}

pub(crate) async fn fleet_reboot(
    socket: Option<&Path>,
    node: &str,
    delay: u64,
) -> anyhow::Result<()> {
    let mut client = DaemonClient::connect(socket).await.map_err(anyhow::Error::msg)?;

    let node_short = truncate(node, 12);
    if delay > 0 {
        eprintln!("  {} rebooting {node_short}… in {delay}s", style("→").cyan());
    } else {
        eprintln!("  {} rebooting {node_short}…", style("→").cyan());
    }

    let result = client.reboot_device(node, delay).await.map_err(anyhow::Error::msg)?;

    let success = result.get("success").and_then(|v| v.as_bool()).unwrap_or(false);

    if success {
        eprintln!("  {} reboot initiated", style("✓").green().bold());
    } else {
        eprintln!("  {} reboot failed", style("✗").red().bold());
    }

    Ok(())
}

pub(crate) async fn fleet_apply(
    socket: Option<&Path>,
    node: &str,
    profile_path: &Path,
    verify: bool,
    timeout: u64,
) -> anyhow::Result<()> {
    // Read and validate profile
    let profile_bytes = std::fs::read(profile_path)
        .map_err(|e| anyhow::anyhow!("failed to read profile: {e}"))?;

    // Quick TOML validation
    let profile_str = std::str::from_utf8(&profile_bytes)
        .map_err(|_| anyhow::anyhow!("profile is not valid UTF-8"))?;
    let _: toml::Value = toml::from_str(profile_str)
        .map_err(|e| anyhow::anyhow!("profile is not valid TOML: {e}"))?;

    // Warn if unsigned and verify enabled
    if verify {
        let parsed: toml::Value = toml::from_str(profile_str).unwrap();
        let has_sig = parsed.get("meta").and_then(|m| m.get("signature")).is_some();
        if !has_sig {
            eprintln!(
                "  {} profile has no signature — verification will fail on remote",
                style("!").yellow().bold()
            );
        }
    }

    let mut client = DaemonClient::connect(socket).await.map_err(anyhow::Error::msg)?;

    let node_short = truncate(node, 12);
    eprintln!("  {} applying profile to {node_short}…", style("→").cyan());
    if verify {
        eprintln!("  {} signature verification enabled", style("✓").dim());
    }

    let result = client
        .fleet_apply(node, &profile_bytes, verify, timeout)
        .await
        .map_err(anyhow::Error::msg)?;

    let verified = result.get("verified").and_then(|v| v.as_bool()).unwrap_or(false);
    let success = result.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
    let exit_code = result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
    let stdout = result.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    let stderr = result.get("stderr").and_then(|v| v.as_str()).unwrap_or("");

    if verified {
        eprintln!("  {} signature verified", style("✓").green().bold());
    }

    if !stdout.is_empty() {
        print!("{stdout}");
        if !stdout.ends_with('\n') {
            println!();
        }
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
        if !stderr.ends_with('\n') {
            eprintln!();
        }
    }

    if success {
        eprintln!("  {} profile applied successfully", style("✓").green().bold());
    } else {
        eprintln!(
            "  {} profile apply failed (exit code {exit_code})",
            style("✗").red().bold()
        );
    }

    Ok(())
}
