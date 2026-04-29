//! CLI command implementations — one-shot IPC calls to the daemon.

use std::path::Path;

use console::style;

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
