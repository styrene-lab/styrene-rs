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
