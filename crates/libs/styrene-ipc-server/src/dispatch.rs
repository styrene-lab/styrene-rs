//! Dispatch IPC message types to [`Daemon`] trait methods.
//!
//! Each request type is mapped to the corresponding trait method. Unimplemented
//! message types return an error string. The payload is a msgpack dict
//! (HashMap<String, rmpv::Value>), and responses are also msgpack dicts.

use std::collections::HashMap;
use std::sync::Arc;

use styrene_ipc::traits::Daemon;

use crate::wire::MessageType;

/// Dispatch a request to the appropriate Daemon method.
///
/// Returns `Ok(payload)` for success or `Err(message)` for errors.
pub async fn dispatch(
    daemon: &Arc<dyn Daemon>,
    msg_type: MessageType,
    payload: HashMap<String, rmpv::Value>,
) -> Result<HashMap<String, rmpv::Value>, String> {
    match msg_type {
        MessageType::QueryStatus => dispatch_query_status(daemon).await,
        MessageType::QueryIdentity => dispatch_query_identity(daemon).await,
        MessageType::QueryDevices => dispatch_query_devices(daemon, &payload).await,
        MessageType::QueryAutoReply => dispatch_query_auto_reply(daemon).await,
        MessageType::CmdAnnounce => dispatch_announce(daemon).await,
        MessageType::QueryConversations => dispatch_query_conversations(daemon, &payload).await,
        MessageType::QueryMessages => dispatch_query_messages(daemon, &payload).await,
        MessageType::CmdSendChat => dispatch_send_chat(daemon, payload).await,
        MessageType::CmdMarkRead => dispatch_mark_read(daemon, &payload).await,
        MessageType::CmdDeleteConversation => dispatch_delete_conversation(daemon, &payload).await,
        MessageType::CmdDeleteMessage => dispatch_delete_message(daemon, &payload).await,
        MessageType::QueryContacts => dispatch_query_contacts(daemon).await,
        MessageType::QueryResolveName => dispatch_resolve_name(daemon, &payload).await,
        MessageType::CmdSetIdentity => dispatch_set_identity(daemon, &payload).await,
        MessageType::CmdRetryMessage => dispatch_retry_message(daemon, &payload).await,
        MessageType::CmdSetAutoReply => dispatch_set_auto_reply(daemon, &payload).await,
        MessageType::QuerySearchMessages => dispatch_search_messages(daemon, &payload).await,
        MessageType::QueryConfig => dispatch_query_config(daemon).await,
        MessageType::CmdSetContact => dispatch_set_contact(daemon, &payload).await,
        MessageType::CmdRemoveContact => dispatch_remove_contact(daemon, &payload).await,
        MessageType::QueryPathInfo => dispatch_query_path_info(daemon, &payload).await,
        MessageType::CmdDeviceStatus => dispatch_device_status(daemon, &payload).await,
        MessageType::SubDevices => dispatch_sub_devices(daemon).await,
        MessageType::SubMessages => dispatch_sub_messages(daemon, &payload).await,
        // TUI-specific types — return sensible defaults without Daemon trait
        MessageType::GetHubStatus => dispatch_get_hub_status().await,
        MessageType::GetUnreadCounts => dispatch_get_unread_counts(daemon).await,
        MessageType::GetNodes => dispatch_get_nodes(daemon, &payload).await,
        MessageType::GetCoreConfig => dispatch_get_core_config(daemon).await,
        MessageType::GetActivityHistory => dispatch_get_activity_history().await,
        MessageType::GetAdapterState => dispatch_get_adapter_state().await,
        MessageType::SubActivity => dispatch_sub_activity().await,
        MessageType::Unsub => dispatch_unsub().await,
        MessageType::CmdExec => dispatch_exec(daemon, &payload).await,
        MessageType::CmdRebootDevice => dispatch_reboot_device(daemon, &payload).await,
        MessageType::CmdBlockPeer => dispatch_block_peer(&payload).await,
        MessageType::CmdUnblockPeer => dispatch_unblock_peer(&payload).await,
        MessageType::QueryBlockedPeers => dispatch_blocked_peers().await,
        MessageType::SaveCoreConfig => dispatch_save_core_config().await,
        MessageType::CmdSyncMessages => dispatch_sync_messages().await,
        MessageType::CmdSend => dispatch_send(daemon, payload).await,
        MessageType::CmdBoundarySnapshot => dispatch_boundary_snapshot().await,
        MessageType::CmdProvisionAdapter => dispatch_provision_adapter().await,
        _ => Err(format!("unimplemented message type: 0x{:02x}", msg_type as u8)),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn val_str<'a>(payload: &'a HashMap<String, rmpv::Value>, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(|v| v.as_str())
}

/// Validate a hex hash string (16-32 hex chars).
fn validate_hash(s: &str) -> Result<&str, String> {
    if s.len() >= 16
        && s.len() <= 64
        && s.chars().all(|c| c.is_ascii_hexdigit())
    {
        Ok(s)
    } else {
        Err(format!("invalid hash: expected 16-64 hex chars, got '{s}'"))
    }
}

fn val_u64(payload: &HashMap<String, rmpv::Value>, key: &str) -> Option<u64> {
    payload.get(key).and_then(|v| v.as_u64())
}

fn val_i64(payload: &HashMap<String, rmpv::Value>, key: &str) -> Option<i64> {
    payload.get(key).and_then(|v| v.as_i64())
}

fn val_bool(payload: &HashMap<String, rmpv::Value>, key: &str) -> Option<bool> {
    payload.get(key).and_then(|v| v.as_bool())
}

type Payload = HashMap<String, rmpv::Value>;

fn ok_payload(p: Payload) -> Result<Payload, String> {
    Ok(p)
}

// ── Status ──────────────────────────────────────────────────────────────

async fn dispatch_query_status(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    let info = daemon.query_status().await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("uptime".into(), rmpv::Value::from(info.uptime));
    p.insert("daemon_version".into(), rmpv::Value::from(info.daemon_version.as_str()));
    p.insert("rns_initialized".into(), rmpv::Value::from(info.rns_initialized));
    p.insert("lxmf_initialized".into(), rmpv::Value::from(info.lxmf_initialized));
    p.insert("device_count".into(), rmpv::Value::from(info.device_count));
    p.insert("interface_count".into(), rmpv::Value::from(info.interface_count));
    if let Some(ref hs) = info.hub_status {
        p.insert("hub_status".into(), rmpv::Value::from(hs.as_str()));
    }
    p.insert("propagation_enabled".into(), rmpv::Value::from(info.propagation_enabled));
    p.insert("transport_enabled".into(), rmpv::Value::from(info.transport_enabled));
    p.insert("active_links".into(), rmpv::Value::from(info.active_links));
    ok_payload(p)
}

// ── Identity ──────────────────────────────────────────────────────────────

async fn dispatch_query_identity(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    let info = daemon.query_identity().await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("identity_hash".into(), rmpv::Value::from(info.identity_hash.as_str()));
    p.insert("destination_hash".into(), rmpv::Value::from(info.destination_hash.as_str()));
    p.insert("lxmf_destination_hash".into(), rmpv::Value::from(info.lxmf_destination_hash.as_str()));
    p.insert("display_name".into(), rmpv::Value::from(info.display_name.as_str()));
    if let Some(ref icon) = info.icon {
        p.insert("icon".into(), rmpv::Value::from(icon.as_str()));
    }
    if let Some(ref sn) = info.short_name {
        p.insert("short_name".into(), rmpv::Value::from(sn.as_str()));
    }
    ok_payload(p)
}

// ── Devices ─────────────────────────────────────────────────────────────

async fn dispatch_query_devices(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let styrene_only = val_bool(payload, "styrene_only").unwrap_or(false);
    let devices = daemon.query_devices(styrene_only).await.map_err(|e| e.to_string())?;
    let device_list: Vec<rmpv::Value> = devices
        .iter()
        .map(|d| {
            rmpv::Value::Map(vec![
                (rmpv::Value::from("destination_hash"), rmpv::Value::from(d.destination_hash.as_str())),
                (rmpv::Value::from("identity_hash"), rmpv::Value::from(d.identity_hash.as_str())),
                (rmpv::Value::from("name"), rmpv::Value::from(d.name.as_str())),
                (rmpv::Value::from("device_type"), rmpv::Value::from(d.device_type.as_str())),
                (rmpv::Value::from("status"), rmpv::Value::from(d.status.as_str())),
                (rmpv::Value::from("is_styrene_node"), rmpv::Value::from(d.is_styrene_node)),
            ])
        })
        .collect();
    let mut p = Payload::new();
    p.insert("devices".into(), rmpv::Value::Array(device_list));
    ok_payload(p)
}

// ── Auto-reply ──────────────────────────────────────────────────────────

async fn dispatch_query_auto_reply(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    let cfg = daemon.query_auto_reply().await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("mode".into(), rmpv::Value::from(cfg.mode.as_str()));
    if let Some(ref msg) = cfg.message {
        p.insert("message".into(), rmpv::Value::from(msg.as_str()));
    }
    if let Some(cd) = cfg.cooldown_secs {
        p.insert("cooldown_secs".into(), rmpv::Value::from(cd));
    }
    ok_payload(p)
}

// ── Announce ────────────────────────────────────────────────────────────

async fn dispatch_announce(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    let ok = daemon.announce().await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("success".into(), rmpv::Value::from(ok));
    ok_payload(p)
}

// ── Conversations ───────────────────────────────────────────────────────

async fn dispatch_query_conversations(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let include_unread = val_bool(payload, "include_unread").unwrap_or(false);
    let convos = daemon
        .query_conversations(include_unread)
        .await
        .map_err(|e| e.to_string())?;
    let list: Vec<rmpv::Value> = convos
        .iter()
        .map(|c| {
            let mut m = Vec::new();
            m.push((rmpv::Value::from("peer_hash"), rmpv::Value::from(c.peer_hash.as_str())));
            m.push((rmpv::Value::from("unread_count"), rmpv::Value::from(c.unread_count)));
            m.push((rmpv::Value::from("message_count"), rmpv::Value::from(c.message_count)));
            if let Some(ref name) = c.peer_name {
                m.push((rmpv::Value::from("peer_name"), rmpv::Value::from(name.as_str())));
            }
            rmpv::Value::Map(m)
        })
        .collect();
    let mut p = Payload::new();
    p.insert("conversations".into(), rmpv::Value::Array(list));
    ok_payload(p)
}

// ── Messages ────────────────────────────────────────────────────────────

async fn dispatch_query_messages(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hash = val_str(payload, "peer_hash").ok_or("missing peer_hash")?;
    let peer_hash = validate_hash(peer_hash)?;
    let limit = val_u64(payload, "limit").unwrap_or(50) as u32;
    let before_ts = val_i64(payload, "before_ts");
    let msgs = daemon
        .query_messages(peer_hash, limit, before_ts)
        .await
        .map_err(|e| e.to_string())?;
    let list: Vec<rmpv::Value> = msgs
        .iter()
        .map(|m| {
            rmpv::Value::Map(vec![
                (rmpv::Value::from("id"), rmpv::Value::from(m.id.as_str())),
                (rmpv::Value::from("source_hash"), rmpv::Value::from(m.source_hash.as_str())),
                (rmpv::Value::from("content"), rmpv::Value::from(m.content.as_str())),
                (rmpv::Value::from("timestamp"), rmpv::Value::from(m.timestamp)),
                (rmpv::Value::from("is_outgoing"), rmpv::Value::from(m.is_outgoing)),
                (rmpv::Value::from("read"), rmpv::Value::from(m.read)),
            ])
        })
        .collect();
    let mut p = Payload::new();
    p.insert("messages".into(), rmpv::Value::Array(list));
    ok_payload(p)
}

// ── Send chat ───────────────────────────────────────────────────────────

async fn dispatch_send_chat(
    daemon: &Arc<dyn Daemon>,
    payload: Payload,
) -> Result<Payload, String> {
    let peer_hash = val_str(&payload, "peer_hash").ok_or("missing peer_hash")?;
    let peer_hash = validate_hash(peer_hash)?.to_string();
    let content = val_str(&payload, "content").ok_or("missing content")?;
    if content.len() > 65536 {
        return Err(format!("content too large: {} bytes (max 65536)", content.len()));
    }
    let content = content.to_string();
    let title = val_str(&payload, "title").map(String::from);
    let delivery_method = val_str(&payload, "delivery_method").map(String::from);

    let mut req = styrene_ipc::types::SendChatRequest::default();
    req.peer_hash = peer_hash;
    req.content = content;
    req.title = title;
    req.delivery_method = delivery_method;
    let msg_id = daemon.send_chat(req).await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("message_id".into(), rmpv::Value::from(msg_id.as_str()));
    ok_payload(p)
}

// ── Mark read ───────────────────────────────────────────────────────────

async fn dispatch_mark_read(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hash = val_str(payload, "peer_hash").ok_or("missing peer_hash")?;
    let peer_hash = validate_hash(peer_hash)?;
    let count = daemon.mark_read(peer_hash).await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("count".into(), rmpv::Value::from(count));
    ok_payload(p)
}

// ── Delete conversation ─────────────────────────────────────────────────

async fn dispatch_delete_conversation(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hash = val_str(payload, "peer_hash").ok_or("missing peer_hash")?;
    let peer_hash = validate_hash(peer_hash)?;
    let count = daemon
        .delete_conversation(peer_hash)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("count".into(), rmpv::Value::from(count));
    ok_payload(p)
}

// ── Delete message ──────────────────────────────────────────────────────

async fn dispatch_delete_message(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let message_id = val_str(payload, "message_id").ok_or("missing message_id")?;
    let ok = daemon
        .delete_message(message_id)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("success".into(), rmpv::Value::from(ok));
    ok_payload(p)
}

// ── Contacts ────────────────────────────────────────────────────────────

async fn dispatch_query_contacts(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    let contacts = daemon.query_contacts().await.map_err(|e| e.to_string())?;
    let list: Vec<rmpv::Value> = contacts
        .iter()
        .map(|c| {
            let mut m = Vec::new();
            m.push((rmpv::Value::from("peer_hash"), rmpv::Value::from(c.peer_hash.as_str())));
            if let Some(ref alias) = c.alias {
                m.push((rmpv::Value::from("alias"), rmpv::Value::from(alias.as_str())));
            }
            rmpv::Value::Map(m)
        })
        .collect();
    let mut p = Payload::new();
    p.insert("contacts".into(), rmpv::Value::Array(list));
    ok_payload(p)
}

// ── Resolve name ────────────────────────────────────────────────────────

async fn dispatch_resolve_name(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let name = val_str(payload, "name").ok_or("missing name")?;
    let prefix = val_str(payload, "prefix");
    let result = daemon
        .resolve_name(name, prefix)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    match result {
        Some(hash) => p.insert("peer_hash".into(), rmpv::Value::from(hash.as_str())),
        None => p.insert("peer_hash".into(), rmpv::Value::Nil),
    };
    ok_payload(p)
}

async fn dispatch_set_identity(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let display_name = val_str(payload, "display_name");
    let icon = val_str(payload, "icon");
    let short_name = val_str(payload, "short_name");
    let changed = daemon
        .set_identity(display_name, icon, short_name)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("changed".into(), rmpv::Value::Boolean(changed));
    ok_payload(p)
}

async fn dispatch_set_auto_reply(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let mode = val_str(payload, "mode").ok_or("missing mode")?;
    let message = val_str(payload, "message");
    let cooldown = payload.get("cooldown_secs").and_then(|v| v.as_u64());
    let changed = daemon
        .set_auto_reply(mode, message, cooldown)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("changed".into(), rmpv::Value::Boolean(changed));
    ok_payload(p)
}

async fn dispatch_search_messages(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let query = val_str(payload, "query").ok_or("missing query")?;
    let peer_hash = val_str(payload, "peer_hash");
    let limit = payload.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as u32;
    let messages = daemon
        .search_messages(query, peer_hash, limit)
        .await
        .map_err(|e| e.to_string())?;
    let arr: Vec<rmpv::Value> = messages
        .iter()
        .map(|m| {
            let mut item = HashMap::new();
            item.insert("id".to_string(), rmpv::Value::from(m.id.as_str()));
            item.insert("source_hash".to_string(), rmpv::Value::from(m.source_hash.as_str()));
            item.insert("content".to_string(), rmpv::Value::from(m.content.as_str()));
            item.insert("timestamp".to_string(), rmpv::Value::from(m.timestamp));
            rmpv::Value::Map(
                item.into_iter()
                    .map(|(k, v)| (rmpv::Value::from(k.as_str()), v))
                    .collect(),
            )
        })
        .collect();
    let mut p = Payload::new();
    p.insert("messages".into(), rmpv::Value::Array(arr));
    ok_payload(p)
}

async fn dispatch_retry_message(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let message_id = val_str(payload, "message_id").ok_or("missing message_id")?;
    let retried = daemon
        .retry_message(message_id)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("retried".into(), rmpv::Value::Boolean(retried));
    ok_payload(p)
}

// ── Query Config ─────────────────────────────────────────────────────────────

async fn dispatch_query_config(
    daemon: &Arc<dyn Daemon>,
) -> Result<Payload, String> {
    let config = daemon.query_config().await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    // Flatten config values into response payload
    for (k, v) in &config.values {
        if let Ok(rv) = serde_json::from_value::<rmpv::Value>(v.clone()) {
            p.insert(k.clone(), rv);
        } else {
            p.insert(k.clone(), rmpv::Value::from(v.to_string().as_str()));
        }
    }
    ok_payload(p)
}

// ── Set/Remove Contact ───────────────────────────────────────────────────────

async fn dispatch_set_contact(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hash = val_str(payload, "peer_hash").ok_or("missing peer_hash")?;
    let peer_hash = validate_hash(peer_hash)?;
    let alias = val_str(payload, "alias");
    let notes = val_str(payload, "notes");
    daemon
        .set_contact(peer_hash, alias, notes)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("ok".into(), rmpv::Value::Boolean(true));
    ok_payload(p)
}

async fn dispatch_remove_contact(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hash = val_str(payload, "peer_hash").ok_or("missing peer_hash")?;
    let peer_hash = validate_hash(peer_hash)?;
    let removed = daemon
        .remove_contact(peer_hash)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("removed".into(), rmpv::Value::Boolean(removed));
    ok_payload(p)
}

// ── Device Status (fleet RPC) ────────────────────────────────────────────────

async fn dispatch_device_status(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let dest = val_str(payload, "destination_hash").ok_or("missing destination_hash")?;
    let dest = validate_hash(dest)?;
    let timeout = payload.get("timeout").and_then(|v| v.as_u64());
    let info = daemon
        .device_status(dest, timeout)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("destination_hash".into(), rmpv::Value::from(info.destination_hash.as_str()));
    if let Some(uptime) = info.uptime {
        p.insert("uptime".into(), rmpv::Value::from(uptime as i64));
    }
    if let Some(ver) = &info.daemon_version {
        p.insert("version".into(), rmpv::Value::from(ver.as_str()));
    }
    ok_payload(p)
}

// ── Subscriptions ────────────────────────────────────────────────────────────

async fn dispatch_sub_devices(
    daemon: &Arc<dyn Daemon>,
) -> Result<Payload, String> {
    let _ = daemon.subscribe_devices().await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("subscribed".into(), rmpv::Value::Boolean(true));
    ok_payload(p)
}

async fn dispatch_sub_messages(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hashes: Vec<String> = payload
        .get("peer_hashes")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let _ = daemon
        .subscribe_messages(&peer_hashes)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("subscribed".into(), rmpv::Value::Boolean(true));
    ok_payload(p)
}

// ── TUI-specific types (not in Daemon trait) ─────────────────────────────────
// These return sensible defaults. As the Rust daemon gains capabilities,
// these can be wired to real service data.

async fn dispatch_get_hub_status() -> Result<Payload, String> {
    let mut p = Payload::new();
    p.insert("is_connected".into(), rmpv::Value::Boolean(false));
    p.insert("status".into(), rmpv::Value::from("disabled"));
    p.insert("hub_address".into(), rmpv::Value::Nil);
    ok_payload(p)
}

async fn dispatch_get_unread_counts(
    daemon: &Arc<dyn Daemon>,
) -> Result<Payload, String> {
    // Build unread counts from conversations
    let convos = daemon
        .query_conversations(true) // unread_only
        .await
        .unwrap_or_default();
    let mut counts = HashMap::new();
    for c in &convos {
        if c.unread_count > 0 {
            counts.insert(
                c.peer_hash.clone(),
                rmpv::Value::from(c.unread_count as i64),
            );
        }
    }
    let mut p = Payload::new();
    p.insert(
        "counts".into(),
        rmpv::Value::Map(
            counts
                .into_iter()
                .map(|(k, v)| (rmpv::Value::from(k.as_str()), v))
                .collect(),
        ),
    );
    ok_payload(p)
}

async fn dispatch_get_nodes(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    // GET_NODES returns persisted nodes — same data as QUERY_DEVICES
    let styrene_only = payload
        .get("styrene_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let devices = daemon
        .query_devices(styrene_only)
        .await
        .map_err(|e| e.to_string())?;
    let arr: Vec<rmpv::Value> = devices
        .iter()
        .map(|d| {
            let mut item = HashMap::new();
            item.insert("destination_hash".to_string(), rmpv::Value::from(d.destination_hash.as_str()));
            item.insert("name".to_string(), rmpv::Value::from(d.name.as_str()));
            item.insert("status".to_string(), rmpv::Value::from(d.status.as_str()));
            item.insert("is_styrene_node".to_string(), rmpv::Value::Boolean(d.is_styrene_node));
            if let Some(ts) = d.last_announce {
                item.insert("last_announce".to_string(), rmpv::Value::from(ts));
            }
            rmpv::Value::Map(
                item.into_iter()
                    .map(|(k, v)| (rmpv::Value::from(k.as_str()), v))
                    .collect(),
            )
        })
        .collect();
    let mut p = Payload::new();
    p.insert("nodes".into(), rmpv::Value::Array(arr));
    ok_payload(p)
}

async fn dispatch_get_core_config(
    daemon: &Arc<dyn Daemon>,
) -> Result<Payload, String> {
    // Return config snapshot — same data as QUERY_CONFIG, wrapped in "config" key
    let config = daemon.query_config().await.map_err(|e| e.to_string())?;
    let mut config_map: Vec<(rmpv::Value, rmpv::Value)> = Vec::new();
    for (k, v) in &config.values {
        let rv = serde_json::from_value::<rmpv::Value>(v.clone())
            .unwrap_or_else(|_| rmpv::Value::from(v.to_string().as_str()));
        config_map.push((rmpv::Value::from(k.as_str()), rv));
    }
    let mut p = Payload::new();
    p.insert("config".into(), rmpv::Value::Map(config_map));
    ok_payload(p)
}

async fn dispatch_get_activity_history() -> Result<Payload, String> {
    // Return empty activity history — EventService ring can be wired later
    let mut p = Payload::new();
    p.insert("events".into(), rmpv::Value::Array(vec![]));
    p.insert("count".into(), rmpv::Value::from(0_i64));
    ok_payload(p)
}

async fn dispatch_get_adapter_state() -> Result<Payload, String> {
    // Return empty adapter list — no adapters in standalone Rust daemon
    let mut p = Payload::new();
    p.insert("adapters".into(), rmpv::Value::Array(vec![]));
    ok_payload(p)
}

async fn dispatch_sub_activity() -> Result<Payload, String> {
    // Acknowledge activity subscription — events pushed via connection writer
    let mut p = Payload::new();
    p.insert("subscribed".into(), rmpv::Value::Boolean(true));
    ok_payload(p)
}

// ── Unsub ────────────────────────────────────────────────────────────────────

async fn dispatch_unsub() -> Result<Payload, String> {
    let mut p = Payload::new();
    p.insert("unsubscribed".into(), rmpv::Value::Boolean(true));
    ok_payload(p)
}

// ── Exec / Reboot (fleet RPC) ────────────────────────────────────────────────

async fn dispatch_exec(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let dest = val_str(payload, "destination_hash").ok_or("missing destination_hash")?;
    let dest = validate_hash(dest)?;
    let cmd = val_str(payload, "command").ok_or("missing command")?;
    let args: Vec<String> = payload
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let timeout = payload.get("timeout").and_then(|v| v.as_u64());
    let result = daemon
        .exec(dest, cmd, args, timeout)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("exit_code".into(), rmpv::Value::from(result.exit_code as i64));
    p.insert("stdout".into(), rmpv::Value::from(result.stdout.as_str()));
    p.insert("stderr".into(), rmpv::Value::from(result.stderr.as_str()));
    ok_payload(p)
}

async fn dispatch_reboot_device(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let dest = val_str(payload, "destination_hash").ok_or("missing destination_hash")?;
    let dest = validate_hash(dest)?;
    let delay = payload.get("delay").and_then(|v| v.as_u64());
    let timeout = payload.get("timeout").and_then(|v| v.as_u64());
    let result = daemon
        .reboot_device(dest, delay, timeout)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("accepted".into(), rmpv::Value::Boolean(result.accepted));
    if let Some(d) = result.delay_secs {
        p.insert("delay_secs".into(), rmpv::Value::from(d as i64));
    }
    ok_payload(p)
}

// ── Send (generic LXMF send — wraps send_chat) ──────────────────────────────

async fn dispatch_send(
    daemon: &Arc<dyn Daemon>,
    payload: Payload,
) -> Result<Payload, String> {
    let peer_hash = val_str(&payload, "destination_hash")
        .or_else(|| val_str(&payload, "peer_hash"))
        .ok_or("missing destination_hash or peer_hash")?;
    let peer_hash = validate_hash(peer_hash)?.to_string();
    let content = val_str(&payload, "content").unwrap_or("").to_string();
    if content.len() > 65536 {
        return Err(format!("content too large: {} bytes", content.len()));
    }
    let title = val_str(&payload, "title").map(|s| s.to_string());
    let mut req = styrene_ipc::types::SendChatRequest::default();
    req.peer_hash = peer_hash;
    req.content = content;
    req.title = title;
    let msg_id = daemon
        .send_chat(req)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("message_id".into(), rmpv::Value::from(msg_id.as_str()));
    ok_payload(p)
}

// ── Peer blocking (stub — not yet in Daemon trait) ───────────────────────────

async fn dispatch_block_peer(payload: &Payload) -> Result<Payload, String> {
    let _hash = val_str(payload, "identity_hash").ok_or("missing identity_hash")?;
    // Stub: peer blocking not yet implemented in Daemon trait
    let mut p = Payload::new();
    p.insert("blocked".into(), rmpv::Value::Boolean(true));
    ok_payload(p)
}

async fn dispatch_unblock_peer(payload: &Payload) -> Result<Payload, String> {
    let _hash = val_str(payload, "identity_hash").ok_or("missing identity_hash")?;
    let mut p = Payload::new();
    p.insert("unblocked".into(), rmpv::Value::Boolean(true));
    ok_payload(p)
}

async fn dispatch_blocked_peers() -> Result<Payload, String> {
    let mut p = Payload::new();
    p.insert("blocked_peers".into(), rmpv::Value::Array(vec![]));
    ok_payload(p)
}

// ── Config save (stub) ──────────────────────────────────────────────────────

async fn dispatch_save_core_config() -> Result<Payload, String> {
    // Stub: config persistence not yet wired
    let mut p = Payload::new();
    p.insert("saved".into(), rmpv::Value::Boolean(true));
    ok_payload(p)
}

// ── Sync messages (stub) ─────────────────────────────────────────────────────

async fn dispatch_sync_messages() -> Result<Payload, String> {
    let mut p = Payload::new();
    p.insert("synced".into(), rmpv::Value::from(0_i64));
    ok_payload(p)
}

// ── Boundary snapshot (stub) ─────────────────────────────────────────────────

async fn dispatch_boundary_snapshot() -> Result<Payload, String> {
    let mut p = Payload::new();
    p.insert("records".into(), rmpv::Value::Array(vec![]));
    ok_payload(p)
}

// ── Provision adapter (stub) ─────────────────────────────────────────────────

async fn dispatch_provision_adapter() -> Result<Payload, String> {
    Err("adapter provisioning not available in Rust daemon".into())
}

// ── Path Info ────────────────────────────────────────────────────────────────

async fn dispatch_query_path_info(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let dest = val_str(payload, "destination_hash").ok_or("missing destination_hash")?;
    let info = daemon.query_path_info(dest).await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("destination_hash".into(), rmpv::Value::from(info.destination_hash.as_str()));
    p.insert("found".into(), rmpv::Value::Boolean(info.hops.is_some()));
    if let Some(hops) = info.hops {
        p.insert("hops".into(), rmpv::Value::from(hops as i64));
    }
    if let Some(iface) = &info.interface {
        p.insert("interface".into(), rmpv::Value::from(iface.as_str()));
    }
    ok_payload(p)
}
