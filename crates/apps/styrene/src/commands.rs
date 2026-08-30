//! CLI command implementations — one-shot IPC calls to the daemon.

use std::path::Path;

use console::style;
use styrene_ipc::types::{SendChatDisposition, SendChatRequest};
use styrene_ipc_client::TunnelStatus;

use crate::ipc_client::connect;

/// Safely truncate a string to at most `n` characters (not bytes).
fn truncate(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

pub(crate) async fn status(socket: Option<&Path>) -> anyhow::Result<()> {
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;
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
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;
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
    delivery_method: &str,
) -> anyhow::Result<()> {
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;
    let mut request = SendChatRequest::default();
    request.peer_hash = destination.into();
    request.content = content.into();
    request.title = title.map(str::to_owned);
    request.delivery_method = Some(delivery_method.into());
    let outcome = client.send_chat_outcome(&request).await.map_err(anyhow::Error::msg)?;
    let msg_id = outcome.message_id;
    let disposition = match outcome.disposition {
        SendChatDisposition::Accepted => "accepted",
        SendChatDisposition::Failed => "failed",
        SendChatDisposition::PaperExported => "paper_exported",
        _ => "unknown",
    };

    eprintln!(
        "  {} {} to {}  (id: {})",
        if disposition == "failed" {
            style("✗").red().bold()
        } else {
            style("✓").green().bold()
        },
        disposition,
        truncate(destination, 12),
        truncate(&msg_id, 8)
    );
    if let Some(uri) = outcome.paper_uri {
        println!("{uri}");
    }

    Ok(())
}

pub(crate) async fn messages(socket: Option<&Path>, peer: &str, limit: u32) -> anyhow::Result<()> {
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;
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
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;
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
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;
    let ok = client.announce().await.map_err(anyhow::Error::msg)?;

    if ok {
        eprintln!("  {} announce sent", style("✓").green().bold());
    } else {
        eprintln!("  {} announce failed", style("✗").red().bold());
    }

    Ok(())
}

pub(crate) async fn config(socket: Option<&Path>) -> anyhow::Result<()> {
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;
    let cfg = client.config().await.map_err(anyhow::Error::msg)?;

    eprintln!();
    eprintln!("  {}", style("styrene config").cyan().bold());
    eprintln!();

    let mut keys: Vec<_> = cfg.values.keys().collect();
    keys.sort();
    for key in keys {
        let val = &cfg.values[key];
        eprintln!("  {key} = {val}");
    }
    eprintln!();

    Ok(())
}

// ── Transport paths ─────────────────────────────────────────────────────────

pub(crate) async fn path_info(socket: Option<&Path>, destination: &str) -> anyhow::Result<()> {
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;
    let info = client.path_info(destination).await.map_err(anyhow::Error::msg)?;
    let found = info.is_some();
    println!("destination={destination}");
    println!("found={found}");
    if let Some(hops) = info.as_ref().and_then(|info| info.hops) {
        println!("hops={hops}");
    }
    if let Some(interface) = info.as_ref().and_then(|info| info.interface.as_deref()) {
        println!("interface={interface}");
    }
    Ok(())
}

// ── Tunnel operations ───────────────────────────────────────────────────────

pub(crate) async fn tunnel_list(socket: Option<&Path>) -> anyhow::Result<()> {
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;

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
                let peer = &t.peer_hash;
                let state = &t.state;
                let endpoint = t.remote_endpoint.as_deref().unwrap_or("");
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
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;
    let status = client.tunnel_status(peer).await.map_err(anyhow::Error::msg)?;

    let peer_short = truncate(peer, 12);
    eprintln!();
    eprintln!("  {} ({peer_short}…)", style("styrene tunnel status").cyan().bold(),);
    eprintln!();
    match status {
        TunnelStatus::Operation(operation) => {
            eprintln!("  peer       {}", operation.peer_hash);
            eprintln!("  operation  {}", operation.operation_id);
            eprintln!("  kind       {}", operation.kind);
            eprintln!("  state      {}", operation.state);
            if let Some(message) = operation.error_message {
                eprintln!("  error      {message}");
            }
        }
        TunnelStatus::Tunnel(tunnel) => {
            eprintln!("  peer       {}", tunnel.peer_hash);
            eprintln!("  state      {}", tunnel.state);
            if !tunnel.backend.is_empty() {
                eprintln!("  backend    {}", tunnel.backend);
            }
            if let Some(endpoint) = tunnel.remote_endpoint {
                eprintln!("  endpoint   {endpoint}");
            }
        }
        _ => eprintln!("  state      unknown"),
    }
    eprintln!();

    Ok(())
}

pub(crate) async fn tunnel_establish(socket: Option<&Path>, peer: &str) -> anyhow::Result<()> {
    tunnel_offer(socket, peer).await
}

pub(crate) async fn tunnel_offer(socket: Option<&Path>, peer: &str) -> anyhow::Result<()> {
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;

    let peer_short = truncate(peer, 12);
    eprintln!("  {} sending tunnel offer to {peer_short}…", style("→").cyan());

    let result = client.tunnel_establish(peer).await.map_err(anyhow::Error::msg)?;

    eprintln!(
        "  {} tunnel operation accepted (id: {}, state: {})",
        style("✓").green().bold(),
        truncate(&result.operation_id, 8),
        result.state
    );

    Ok(())
}

pub(crate) async fn tunnel_teardown(socket: Option<&Path>, peer: &str) -> anyhow::Result<()> {
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;

    let peer_short = truncate(peer, 12);
    eprintln!("  {} tearing down tunnel to {peer_short}…", style("→").cyan());

    let success = client.tunnel_teardown(peer).await.map_err(anyhow::Error::msg)?;
    if success {
        eprintln!("  {} tunnel torn down", style("✓").green().bold());
    } else {
        eprintln!("  {} tunnel teardown failed", style("✗").red().bold());
    }

    Ok(())
}

// ── Fleet operations ────────────────────────────────────────────────────────

pub(crate) async fn fleet_status(
    socket: Option<&Path>,
    node: Option<&str>,
    timeout: u64,
) -> anyhow::Result<()> {
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;

    if let Some(dest) = node {
        let node_short = truncate(dest, 12);
        eprintln!();
        eprintln!("  {} (querying {node_short}…)", style("styrene fleet status").cyan().bold(),);

        let result = client.device_status(dest, timeout).await.map_err(anyhow::Error::msg)?;

        eprintln!();
        eprintln!("  destination_hash: {}", result.destination_hash);
        if let Some(uptime) = result.uptime {
            eprintln!("  uptime: {uptime}");
        }
        if let Some(version) = result.daemon_version {
            eprintln!("  version: {version}");
        }
        for (key, value) in result.extra {
            eprintln!("  {key}: {value}");
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
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;

    let node_short = truncate(node, 12);
    eprintln!("  {} exec on {node_short}…: {cmd} {}", style("→").cyan(), args.join(" "));

    let result = client.exec(node, cmd, args, timeout).await.map_err(anyhow::Error::msg)?;

    let exit_code = result.exit_code;
    let stdout = result.stdout;
    let stderr = result.stderr;

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
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;

    let node_short = truncate(node, 12);
    if delay > 0 {
        eprintln!("  {} rebooting {node_short}… in {delay}s", style("→").cyan());
    } else {
        eprintln!("  {} rebooting {node_short}…", style("→").cyan());
    }

    let result = client.reboot_device(node, delay).await.map_err(anyhow::Error::msg)?;

    if result.accepted {
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
    // Issue 8: Clamp timeout to reasonable bounds (10s to 1h)
    let timeout = timeout.clamp(10, 3600);

    // Read and validate profile
    let profile_bytes =
        std::fs::read(profile_path).map_err(|e| anyhow::anyhow!("failed to read profile: {e}"))?;

    // Quick TOML validation
    let profile_str = std::str::from_utf8(&profile_bytes)
        .map_err(|_| anyhow::anyhow!("profile is not valid UTF-8"))?;
    let _: toml::Value = toml::from_str(profile_str)
        .map_err(|e| anyhow::anyhow!("profile is not valid TOML: {e}"))?;

    // Warn if unsigned and verify enabled
    if verify {
        let parsed: toml::Value = toml::from_str(profile_str).expect("valid test profile TOML");
        let has_sig = parsed.get("meta").and_then(|m| m.get("signature")).is_some();
        if !has_sig {
            eprintln!(
                "  {} profile has no signature — verification will fail on remote",
                style("!").yellow().bold()
            );
        }
    }

    let client = connect(socket).await.map_err(anyhow::Error::msg)?;

    let node_short = truncate(node, 12);
    eprintln!("  {} applying profile to {node_short}…", style("→").cyan());
    if verify {
        eprintln!("  {} signature verification enabled", style("✓").dim());
    }

    let result = client
        .fleet_apply(node, &profile_bytes, verify, timeout)
        .await
        .map_err(anyhow::Error::msg)?;

    let verified = result.verified;
    let success = result.success;
    let exit_code = result.exit_code;
    let stdout = result.stdout;
    let stderr = result.stderr;

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
        eprintln!("  {} profile apply failed (exit code {exit_code})", style("✗").red().bold());
    }

    Ok(())
}

pub(crate) async fn fleet_grant(
    socket: Option<&Path>,
    node: &str,
    role: &str,
    label: Option<&str>,
    grants: &[String],
) -> anyhow::Result<()> {
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;

    let node_short = truncate(node, 12);
    eprintln!("  {} granting {role} to {node_short}…", style("→").cyan());

    let success = client
        .fleet_grant(node, role, label.unwrap_or(""), grants)
        .await
        .map_err(anyhow::Error::msg)?;

    if success {
        eprintln!("  {} role granted: {role}", style("✓").green().bold());
    } else {
        eprintln!("  {} grant failed", style("✗").red().bold());
    }

    Ok(())
}

pub(crate) async fn fleet_revoke(socket: Option<&Path>, node: &str) -> anyhow::Result<()> {
    let client = connect(socket).await.map_err(anyhow::Error::msg)?;

    let node_short = truncate(node, 12);
    eprintln!("  {} revoking role from {node_short}…", style("→").cyan());

    let success = client.fleet_revoke(node).await.map_err(anyhow::Error::msg)?;

    if success {
        eprintln!("  {} role revoked", style("✓").green().bold());
    } else {
        eprintln!("  {} revoke failed (identity not in roster)", style("✗").red().bold());
    }

    Ok(())
}
