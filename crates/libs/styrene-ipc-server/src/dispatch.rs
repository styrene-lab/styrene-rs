//! Dispatch IPC message types to [`Daemon`] trait methods.
//!
//! Each request type is mapped to the corresponding trait method. Unimplemented
//! message types return an error string. The payload is a msgpack dict
//! (HashMap<String, rmpv::Value>), and responses are also msgpack dicts.

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
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
    dispatch_for_connection(daemon, msg_type, payload, 0).await
}

/// Dispatch with metadata scoped to the physical IPC connection.
pub async fn dispatch_for_connection(
    daemon: &Arc<dyn Daemon>,
    msg_type: MessageType,
    payload: HashMap<String, rmpv::Value>,
    connection_generation: u64,
) -> Result<HashMap<String, rmpv::Value>, String> {
    match msg_type {
        MessageType::QueryStatus => dispatch_query_status(daemon, connection_generation).await,
        MessageType::QueryMobileDiagnostics => dispatch_mobile_diagnostics(daemon).await,
        MessageType::CmdExportMobileDiagnostics => dispatch_export_mobile_diagnostics(daemon).await,
        MessageType::QueryPropagation => dispatch_query_propagation(daemon, &payload).await,
        MessageType::QueryStandardPropagation => {
            dispatch_query_standard_propagation(daemon, connection_generation).await
        }
        MessageType::QueryLinks => dispatch_query_links(daemon, connection_generation).await,
        MessageType::QueryRequest => {
            dispatch_query_request(daemon, &payload, connection_generation).await
        }
        MessageType::QueryRequests => dispatch_query_requests(daemon, connection_generation).await,
        MessageType::CmdRequestStart => {
            dispatch_start_request(daemon, &payload, connection_generation).await
        }
        MessageType::CmdRequestCancel => {
            dispatch_cancel_request(daemon, &payload, connection_generation).await
        }
        MessageType::QueryNetworkOperation => {
            dispatch_query_network_operation(daemon, &payload, connection_generation).await
        }
        MessageType::CmdNetworkOperationStart => {
            dispatch_start_network_operation(daemon, &payload, connection_generation).await
        }
        MessageType::CmdNetworkOperationCancel => {
            dispatch_cancel_network_operation(daemon, &payload, connection_generation).await
        }
        MessageType::QueryResources => {
            dispatch_query_resources(daemon, connection_generation).await
        }
        MessageType::CmdResourceCancel => dispatch_cancel_resource(daemon, &payload).await,
        MessageType::QueryAttachmentTransfer => {
            dispatch_query_attachment_transfer(daemon, &payload).await
        }
        MessageType::CmdAttachmentTransferCancel => {
            dispatch_cancel_attachment_transfer(daemon, &payload).await
        }
        MessageType::QueryIdentity => dispatch_query_identity(daemon).await,
        MessageType::QueryDevices => dispatch_query_devices(daemon, &payload).await,
        MessageType::QueryAutoReply => dispatch_query_auto_reply(daemon).await,
        MessageType::CmdAnnounce => dispatch_announce(daemon).await,
        MessageType::QueryConversations => dispatch_query_conversations(daemon, &payload).await,
        MessageType::CmdStartConversation => dispatch_start_conversation(daemon, &payload).await,
        MessageType::QueryMessages => dispatch_query_messages(daemon, &payload).await,
        MessageType::QueryMessage => dispatch_query_message(daemon, &payload).await,
        MessageType::CmdSendChat => dispatch_send_chat(daemon, payload, false).await,
        MessageType::CmdSendChatOutcome => dispatch_send_chat(daemon, payload, true).await,
        MessageType::QueryDraft => dispatch_query_draft(daemon, &payload).await,
        MessageType::CmdSetDraft => dispatch_set_draft(daemon, &payload).await,
        MessageType::CmdClearDraft => dispatch_clear_draft(daemon, &payload).await,
        MessageType::CmdMarkRead => dispatch_mark_read(daemon, &payload).await,
        MessageType::CmdDeleteConversation => dispatch_delete_conversation(daemon, &payload).await,
        MessageType::CmdDeleteMessage => dispatch_delete_message(daemon, &payload).await,
        MessageType::QueryContacts => dispatch_query_contacts(daemon).await,
        MessageType::QueryResolveName => dispatch_resolve_name(daemon, &payload).await,
        MessageType::CmdSetIdentity => dispatch_set_identity(daemon, &payload).await,
        MessageType::CmdRetryMessage => dispatch_retry_message(daemon, &payload).await,
        MessageType::CmdCancelMessage => dispatch_cancel_message(daemon, &payload).await,
        MessageType::CmdSetAutoReply => dispatch_set_auto_reply(daemon, &payload).await,
        MessageType::QuerySearchMessages => dispatch_search_messages(daemon, &payload).await,
        MessageType::QueryConfig => dispatch_query_config(daemon).await,
        MessageType::CmdSetContact => dispatch_set_contact(daemon, &payload).await,
        MessageType::CmdRemoveContact => dispatch_remove_contact(daemon, &payload).await,
        MessageType::CmdPinConversation => {
            dispatch_conversation_flag(daemon, &payload, "pin").await
        }
        MessageType::CmdUnpinConversation => {
            dispatch_conversation_flag(daemon, &payload, "unpin").await
        }
        MessageType::CmdMuteConversation => {
            dispatch_conversation_flag(daemon, &payload, "mute").await
        }
        MessageType::CmdUnmuteConversation => {
            dispatch_conversation_flag(daemon, &payload, "unmute").await
        }
        MessageType::QueryPathInfo => {
            dispatch_query_path_info(daemon, &payload, connection_generation).await
        }
        MessageType::QueryPathTable => {
            dispatch_query_path_table(daemon, connection_generation).await
        }
        MessageType::QueryInterfaceStats => {
            dispatch_query_interface_stats(daemon, connection_generation).await
        }
        MessageType::CmdRemoteInbox => dispatch_remote_inbox(daemon, &payload).await,
        MessageType::CmdRemoteMessages => dispatch_remote_messages(daemon, &payload).await,
        MessageType::CmdSelfUpdate => dispatch_self_update(daemon, &payload).await,
        MessageType::CmdPqcStatus => {
            // PQC status — return stub until post-quantum is implemented
            let mut p = Payload::new();
            p.insert("pqc_available".into(), rmpv::Value::Boolean(false));
            p.insert("pqc_active".into(), rmpv::Value::Boolean(false));
            ok_payload(p)
        }
        MessageType::QueryAttachment => dispatch_query_attachment(daemon, &payload).await,
        MessageType::QueryPage => {
            dispatch_query_page(daemon, &payload, connection_generation).await
        }
        MessageType::CmdPageNavigate => {
            dispatch_page_navigate(daemon, &payload, connection_generation).await
        }
        MessageType::CmdPageDisconnect => {
            dispatch_page_disconnect(daemon, &payload, connection_generation).await
        }
        MessageType::CmdFileDownloadStart => {
            dispatch_file_download_start(daemon, &payload, connection_generation).await
        }
        MessageType::QueryFileDownload => {
            dispatch_file_download_query(daemon, &payload, connection_generation).await
        }
        MessageType::CmdFileDownloadCancel => {
            dispatch_file_download_cancel(daemon, &payload, connection_generation).await
        }
        MessageType::CmdFileDownloadSave => {
            dispatch_file_download_save(daemon, &payload, connection_generation).await
        }
        MessageType::CmdPageListSites => dispatch_list_pages(daemon, &payload).await,
        MessageType::QueryPageServerStatus
        | MessageType::CmdPageGetCached
        | MessageType::CmdPageSaveSite
        | MessageType::CmdPageRemoveSite
        | MessageType::CmdPageCrawlSite
        | MessageType::CmdPageRegenerate => Err("page management not yet implemented".into()),
        MessageType::CmdTerminalOpen => dispatch_terminal_open(daemon, &payload).await,
        MessageType::CmdTerminalInput => dispatch_terminal_input(daemon, &payload).await,
        MessageType::CmdTerminalClose => dispatch_terminal_close(daemon, &payload).await,
        MessageType::CmdTerminalResize => dispatch_terminal_resize(daemon, &payload).await,
        MessageType::CmdDatalinkEstablish
        | MessageType::CmdDatalinkTeardown
        | MessageType::CmdDatalinkQuery
        | MessageType::CmdDatalinkInfo
        | MessageType::CmdDatalinkStatus
        | MessageType::CmdDatalinkMeta
        | MessageType::CmdDatalinkSpeedtest => {
            // Datalink management — P3, not yet implemented
            Err("datalink management not yet implemented".into())
        }
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
        MessageType::SubLinks => dispatch_sub_links().await,
        MessageType::SubNetworkOperations => ok_payload(Payload::new()),
        MessageType::SubResources => ok_payload(Payload::new()),
        // Unsub is handled in connection.rs before dispatch — this is unreachable
        MessageType::Unsub => ok_payload(Payload::new()),
        MessageType::CmdExec => dispatch_exec(daemon, &payload).await,
        MessageType::CmdRebootDevice => dispatch_reboot_device(daemon, &payload).await,
        MessageType::CmdBlockPeer => dispatch_block_peer(daemon, &payload).await,
        MessageType::CmdUnblockPeer => dispatch_unblock_peer(daemon, &payload).await,
        MessageType::QueryBlockedPeers => dispatch_blocked_peers(daemon).await,
        MessageType::SaveCoreConfig => dispatch_save_core_config(daemon).await,
        MessageType::CmdSyncMessages => dispatch_sync_messages().await,
        MessageType::CmdSend => dispatch_send(daemon, &payload).await,
        MessageType::CmdBoundarySnapshot => dispatch_boundary_snapshot().await,
        MessageType::CmdProvisionAdapter => dispatch_provision_adapter().await,
        MessageType::QueryTunnels => dispatch_query_tunnels(daemon).await,
        MessageType::QueryTunnelStatus => dispatch_query_tunnel_status(daemon, &payload).await,
        MessageType::CmdTunnelTeardown => dispatch_tunnel_teardown(daemon, &payload).await,
        MessageType::CmdFleetApply => dispatch_fleet_apply(daemon, &payload).await,
        MessageType::CmdTunnelEstablish => dispatch_tunnel_establish(daemon, &payload).await,
        MessageType::CmdFleetGrant => dispatch_fleet_grant(daemon, &payload).await,
        MessageType::CmdFleetRevoke => dispatch_fleet_revoke(daemon, &payload).await,
        _ => Err(format!("unimplemented message type: 0x{:02x}", msg_type as u8)),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn val_str<'a>(payload: &'a HashMap<String, rmpv::Value>, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(|v| v.as_str())
}

fn validate_peer_hash(s: &str) -> Result<&str, String> {
    if s.len() != 32
        || s.bytes().any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err("peer hash must be exactly 32 lowercase hexadecimal characters".into());
    }
    Ok(s)
}

fn validate_message_id(s: &str) -> Result<&str, String> {
    if s.is_empty()
        || s.len() > 128
        || s.bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("message ID must be 1..=128 ASCII identifier characters".into());
    }
    Ok(s)
}

fn val_u64(payload: &HashMap<String, rmpv::Value>, key: &str) -> Option<u64> {
    payload.get(key).and_then(|v| v.as_u64())
}

fn message_query_limit(payload: &HashMap<String, rmpv::Value>) -> Result<u32, String> {
    let limit = val_u64(payload, "limit").unwrap_or(50);
    if limit > u64::from(styrene_ipc::types::MAX_MESSAGE_QUERY_LIMIT) {
        return Err(format!(
            "message query limit {limit} exceeds maximum {}",
            styrene_ipc::types::MAX_MESSAGE_QUERY_LIMIT
        ));
    }
    u32::try_from(limit).map_err(|_| "message query limit exceeds u32 range".into())
}

fn page_limit(payload: &Payload) -> Result<u32, String> {
    let limit = match payload.get("limit") {
        None => 50,
        Some(value) => value.as_u64().ok_or("page limit must be an unsigned integer")?,
    };
    if !(1..=u64::from(styrene_ipc::types::MAX_MESSAGE_QUERY_LIMIT)).contains(&limit) {
        return Err(format!(
            "page limit must be between 1 and {}",
            styrene_ipc::types::MAX_MESSAGE_QUERY_LIMIT
        ));
    }
    u32::try_from(limit).map_err(|_| "page limit exceeds u32 range".into())
}

fn page_cursor<'a>(payload: &'a Payload, key: &str) -> Result<Option<&'a str>, String> {
    let Some(value) = payload.get(key) else {
        return Ok(None);
    };
    let cursor = value.as_str().ok_or_else(|| format!("{key} must be a string"))?;
    if cursor.is_empty() || cursor.len() > styrene_ipc::types::MAX_PAGE_CURSOR_LENGTH {
        return Err(format!(
            "{key} length must be between 1 and {}",
            styrene_ipc::types::MAX_PAGE_CURSOR_LENGTH
        ));
    }
    Ok(Some(cursor))
}

fn val_bool(payload: &HashMap<String, rmpv::Value>, key: &str) -> Option<bool> {
    payload.get(key).and_then(|v| v.as_bool())
}

type Payload = HashMap<String, rmpv::Value>;

fn ok_payload(p: Payload) -> Result<Payload, String> {
    let encoded = rmp_serde::to_vec(&p).map_err(|error| format!("encode IPC response: {error}"))?;
    if encoded.len() > crate::wire::MAX_PAYLOAD_SIZE {
        return Err(format!(
            "IPC response payload is {} bytes; maximum is {} (reduce page limit)",
            encoded.len(),
            crate::wire::MAX_PAYLOAD_SIZE
        ));
    }
    Ok(p)
}

fn serialized_value<T: serde::Serialize>(value: &T) -> Result<rmpv::Value, String> {
    let json =
        serde_json::to_value(value).map_err(|error| format!("encode typed IPC value: {error}"))?;
    rmpv::ext::to_value(json).map_err(|error| format!("encode typed IPC value: {error}"))
}

fn typed_ipc_error(error: styrene_ipc::IpcError) -> String {
    serde_json::to_string(&error).unwrap_or_else(|_| error.to_string())
}

fn invalid_dispatch(message: impl Into<String>) -> String {
    typed_ipc_error(styrene_ipc::IpcError::invalid_request(message))
}

fn required_str<'a>(payload: &'a Payload, key: &str) -> Result<&'a str, String> {
    val_str(payload, key).ok_or_else(|| invalid_dispatch(format!("missing {key}")))
}

fn required_peer_hash<'a>(payload: &'a Payload, key: &str) -> Result<&'a str, String> {
    validate_peer_hash(required_str(payload, key)?).map_err(invalid_dispatch)
}

fn required_message_id(payload: &Payload) -> Result<&str, String> {
    validate_message_id(required_str(payload, "message_id")?).map_err(invalid_dispatch)
}

fn add_outcome(
    payload: &mut Payload,
    outcome: &styrene_ipc::types::MessagingOperationOutcome,
) -> Result<(), String> {
    payload.insert("outcome".into(), serialized_value(outcome)?);
    Ok(())
}

async fn dispatch_query_resources(
    daemon: &Arc<dyn Daemon>,
    connection_generation: u64,
) -> Result<Payload, String> {
    let resources = daemon.resource_transfers().await.map_err(|error| error.to_string())?;
    let resources = resources
        .into_iter()
        .map(|resource| resource_info_value(resource, connection_generation))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Payload::from([("resources".into(), rmpv::Value::Array(resources))]))
}

async fn dispatch_cancel_resource(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let resource_hash = val_str(payload, "resource_hash").ok_or("missing resource_hash")?;
    let accepted =
        daemon.cancel_resource(resource_hash).await.map_err(|error| error.to_string())?;
    Ok(Payload::from([("accepted".into(), rmpv::Value::from(accepted))]))
}

pub(crate) fn resource_info_payload(
    resource: styrene_ipc::types::ResourceTransferInfo,
    connection_generation: u64,
) -> Result<Payload, String> {
    let value = resource_info_value(resource, connection_generation)?;
    value.as_map().ok_or_else(|| "resource encoding was not a map".to_string()).map(|entries| {
        entries
            .iter()
            .filter_map(|(key, value)| key.as_str().map(|key| (key.to_string(), value.clone())))
            .collect()
    })
}

fn resource_info_value(
    mut resource: styrene_ipc::types::ResourceTransferInfo,
    connection_generation: u64,
) -> Result<rmpv::Value, String> {
    if connection_generation != 0 {
        resource.observation.connection_generation = Some(connection_generation);
    }
    let json = serde_json::to_value(resource).map_err(|error| error.to_string())?;
    serde_json::from_value(json).map_err(|error| error.to_string())
}

async fn dispatch_start_request(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    connection_generation: u64,
) -> Result<Payload, String> {
    let mut request = styrene_ipc::types::StartRequestInfo::default();
    request.link_id = val_str(payload, "link_id").ok_or("missing link_id")?.to_string();
    request.path = val_str(payload, "path").ok_or("missing path")?.to_string();
    request.data = payload
        .get("data")
        .and_then(|value| value.as_slice())
        .ok_or("missing binary data")?
        .to_vec();
    request.timeout_ms = val_u64(payload, "timeout_ms").ok_or("missing timeout_ms")?;
    request.max_response_size =
        val_u64(payload, "max_response_size").ok_or("missing max_response_size")?;
    request_info_payload(
        daemon.start_request(request).await.map_err(|error| error.to_string())?,
        connection_generation,
    )
}

async fn dispatch_query_request(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    connection_generation: u64,
) -> Result<Payload, String> {
    let request_id = val_str(payload, "request_id").ok_or("missing request_id")?;
    let receipt = daemon
        .request_receipt(request_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("request receipt not found")?;
    request_info_payload(receipt, connection_generation)
}

async fn dispatch_query_requests(
    daemon: &Arc<dyn Daemon>,
    connection_generation: u64,
) -> Result<Payload, String> {
    let receipts = daemon.request_receipts().await.map_err(|error| error.to_string())?;
    let values = receipts
        .into_iter()
        .map(|receipt| request_info_value(receipt, connection_generation))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Payload::from([("requests".into(), rmpv::Value::Array(values))]))
}

async fn dispatch_cancel_request(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    connection_generation: u64,
) -> Result<Payload, String> {
    let request_id = val_str(payload, "request_id").ok_or("missing request_id")?;
    request_info_payload(
        daemon.cancel_request(request_id).await.map_err(|error| error.to_string())?,
        connection_generation,
    )
}

async fn dispatch_start_network_operation(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    connection_generation: u64,
) -> Result<Payload, String> {
    use styrene_ipc::types::{NetworkOperationKind, StartNetworkOperationInfo};

    let kind = match val_str(payload, "kind").ok_or("missing kind")? {
        "announce" => NetworkOperationKind::Announce,
        "path_request" => NetworkOperationKind::PathRequest,
        "probe" => NetworkOperationKind::Probe,
        "link_open" => NetworkOperationKind::LinkOpen,
        "link_close" => NetworkOperationKind::LinkClose,
        other => return Err(format!("unknown network operation kind: {other}")),
    };
    let mut request = StartNetworkOperationInfo::default();
    request.kind = kind;
    request.destination_hash = val_str(payload, "destination_hash")
        .map(validate_peer_hash)
        .transpose()?
        .map(str::to_string);
    request.link_id = val_str(payload, "link_id").map(str::to_string);
    request.timeout_ms = val_u64(payload, "timeout_ms").ok_or("missing timeout_ms")?;
    network_operation_payload(
        daemon.start_network_operation(request).await.map_err(|error| error.to_string())?,
        connection_generation,
    )
}

async fn dispatch_query_network_operation(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    connection_generation: u64,
) -> Result<Payload, String> {
    if let Some(operation_id) = val_str(payload, "operation_id") {
        let operation = daemon
            .network_operation(operation_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("network operation not found")?;
        return network_operation_payload(operation, connection_generation);
    }
    let operations = daemon.network_operations().await.map_err(|error| error.to_string())?;
    let operations = operations
        .into_iter()
        .map(|operation| {
            network_operation_payload(operation, connection_generation).map(|payload| {
                rmpv::Value::Map(
                    payload
                        .into_iter()
                        .map(|(key, value)| (rmpv::Value::from(key), value))
                        .collect(),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Payload::from([("operations".into(), rmpv::Value::Array(operations))]))
}

async fn dispatch_cancel_network_operation(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    connection_generation: u64,
) -> Result<Payload, String> {
    let operation_id = val_str(payload, "operation_id").ok_or("missing operation_id")?;
    network_operation_payload(
        daemon.cancel_network_operation(operation_id).await.map_err(|error| error.to_string())?,
        connection_generation,
    )
}

pub(crate) fn network_operation_payload(
    operation: styrene_ipc::types::NetworkOperationInfo,
    connection_generation: u64,
) -> Result<Payload, String> {
    use styrene_ipc::types::NetworkOperationOutcome;

    let mut payload = Payload::from([
        ("operation_id".into(), rmpv::Value::from(operation.operation_id.as_str())),
        ("kind".into(), rmpv::Value::from(operation.kind.as_str())),
        ("started_unix_ms".into(), rmpv::Value::from(operation.started_unix_ms)),
        ("deadline_unix_ms".into(), rmpv::Value::from(operation.deadline_unix_ms)),
        ("cancellable".into(), rmpv::Value::from(operation.cancellable)),
        ("progress".into(), rmpv::Value::from(operation.progress.as_str())),
        ("source".into(), rmpv::Value::from(operation.observation.source.as_str())),
        ("connection_generation".into(), rmpv::Value::from(connection_generation)),
    ]);
    if let Some(destination) = operation.destination_hash {
        payload.insert("destination_hash".into(), rmpv::Value::from(destination));
    }
    if let Some(link_id) = operation.link_id {
        payload.insert("link_id".into(), rmpv::Value::from(link_id));
    }
    if let Some(outcome) = operation.outcome {
        let outcome = match outcome {
            NetworkOperationOutcome::Succeeded => "succeeded",
            NetworkOperationOutcome::Dispatched => "dispatched",
            NetworkOperationOutcome::TimedOut => "timed_out",
            NetworkOperationOutcome::Denied => "denied",
            NetworkOperationOutcome::Unavailable => "unavailable",
            NetworkOperationOutcome::Cancelled => "cancelled",
            NetworkOperationOutcome::Failed => "failed",
            _ => "unknown",
        };
        payload.insert("outcome".into(), rmpv::Value::from(outcome));
    }
    if let Some(detail) = operation.detail {
        payload.insert("detail".into(), rmpv::Value::from(detail));
    }
    if let Some(rtt_ms) = operation.rtt_ms {
        payload.insert("rtt_ms".into(), rmpv::Value::F64(rtt_ms));
    }
    if let Some(observed_at) = operation.observation.observed_at {
        payload.insert("observed_at".into(), rmpv::Value::from(observed_at));
    }
    if let Some(correlation_id) = operation.observation.correlation_id {
        payload.insert("correlation_id".into(), rmpv::Value::from(correlation_id));
    }
    Ok(payload)
}

pub(crate) fn request_info_payload(
    info: styrene_ipc::types::RequestObservationInfo,
    connection_generation: u64,
) -> Result<Payload, String> {
    let value = request_info_value(info, connection_generation)?;
    value.as_map().ok_or_else(|| "request receipt encoding was not a map".to_string()).map(
        |entries| {
            entries
                .iter()
                .filter_map(|(key, value)| key.as_str().map(|key| (key.to_string(), value.clone())))
                .collect()
        },
    )
}

fn request_info_value(
    mut info: styrene_ipc::types::RequestObservationInfo,
    connection_generation: u64,
) -> Result<rmpv::Value, String> {
    let response = info.response.take();
    if connection_generation != 0 {
        info.observation.connection_generation = Some(connection_generation);
    }
    let json = serde_json::to_value(info).map_err(|error| error.to_string())?;
    let mut value: rmpv::Value = serde_json::from_value(json).map_err(|error| error.to_string())?;
    if let Some(response) = response {
        let rmpv::Value::Map(entries) = &mut value else {
            return Err("request receipt encoding was not a map".to_string());
        };
        let encoded = entries
            .iter_mut()
            .find(|(key, _)| key.as_str() == Some("response"))
            .ok_or_else(|| "request receipt encoding omitted response".to_string())?;
        encoded.1 = rmpv::Value::Binary(response);
    }
    Ok(value)
}

// ── Status ──────────────────────────────────────────────────────────────

fn capability_failure_code(code: styrene_ipc::types::CapabilityFailureCode) -> &'static str {
    match code {
        styrene_ipc::types::CapabilityFailureCode::Unavailable => "unavailable",
        styrene_ipc::types::CapabilityFailureCode::Unauthorized => "unauthorized",
        styrene_ipc::types::CapabilityFailureCode::Degraded => "degraded",
        styrene_ipc::types::CapabilityFailureCode::Unverified => "unverified",
        _ => "unknown",
    }
}

async fn dispatch_query_status(
    daemon: &Arc<dyn Daemon>,
    connection_generation: u64,
) -> Result<Payload, String> {
    let mut info = daemon.query_status().await.map_err(|e| e.to_string())?;
    if connection_generation != 0 {
        info.connection_generation = Some(connection_generation);
    }
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
    p.insert(
        "standard_lxmf_propagation_destination_registered".into(),
        rmpv::Value::from(info.standard_lxmf_propagation_destination_registered),
    );
    p.insert(
        "standard_lxmf_propagation_active".into(),
        rmpv::Value::from(info.standard_lxmf_propagation_active),
    );
    p.insert("propagation_count".into(), rmpv::Value::from(info.propagation_count as i64));
    p.insert(
        "propagation_size_bytes".into(),
        rmpv::Value::from(info.propagation_size_bytes as i64),
    );
    p.insert("transport_enabled".into(), rmpv::Value::from(info.transport_enabled));
    p.insert("active_links".into(), rmpv::Value::from(info.active_links));
    if let Some(capabilities) = info.active_capabilities {
        let degraded = capabilities
            .degraded
            .into_iter()
            .map(|capability| {
                rmpv::Value::Map(vec![
                    (rmpv::Value::from("id"), rmpv::Value::from(capability.id)),
                    (rmpv::Value::from("reason"), rmpv::Value::from(capability.reason)),
                    (
                        rmpv::Value::from("reason_code"),
                        rmpv::Value::from(capability_failure_code(capability.reason_code)),
                    ),
                ])
            })
            .collect();
        let failures = capabilities
            .failures
            .into_iter()
            .map(|failure| {
                rmpv::Value::Map(vec![
                    (rmpv::Value::from("id"), rmpv::Value::from(failure.id)),
                    (
                        rmpv::Value::from("code"),
                        rmpv::Value::from(capability_failure_code(failure.code)),
                    ),
                    (rmpv::Value::from("retryable"), rmpv::Value::from(failure.retryable)),
                ])
            })
            .collect();
        let mut capability_map = vec![
            (rmpv::Value::from("version"), rmpv::Value::from(capabilities.version)),
            (
                rmpv::Value::from("runtime"),
                rmpv::Value::Array(
                    capabilities.runtime.into_iter().map(rmpv::Value::from).collect(),
                ),
            ),
            (rmpv::Value::from("degraded"), rmpv::Value::Array(degraded)),
            (rmpv::Value::from("failures"), rmpv::Value::Array(failures)),
            (
                rmpv::Value::from("authorized_operations"),
                rmpv::Value::Array(
                    capabilities.authorized_operations.into_iter().map(rmpv::Value::from).collect(),
                ),
            ),
        ];
        if let Some(generation) = capabilities.generation {
            capability_map.push((rmpv::Value::from("generation"), rmpv::Value::from(generation)));
        }
        p.insert("active_capabilities".into(), rmpv::Value::Map(capability_map));
    }
    if let Some(generation) = info.connection_generation {
        p.insert("connection_generation".into(), rmpv::Value::from(generation));
    }
    ok_payload(p)
}

async fn dispatch_mobile_diagnostics(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    let snapshot = daemon.mobile_diagnostics().await.map_err(typed_ipc_error)?;
    if snapshot.events.len() > styrene_ipc::types::MOBILE_DIAGNOSTIC_MAX_EVENTS as usize {
        return Err("mobile diagnostic snapshot exceeds event limit".into());
    }
    let events = snapshot.events.iter().map(mobile_diagnostic_event_value).collect();
    ok_payload(Payload::from([
        ("schema_version".into(), snapshot.schema_version.into()),
        ("backend_revision".into(), snapshot.backend_revision.into()),
        ("first_sequence".into(), optional_value(snapshot.first_sequence)),
        ("last_sequence".into(), optional_value(snapshot.last_sequence)),
        ("event_count".into(), snapshot.event_count.into()),
        ("retained_bytes".into(), snapshot.retained_bytes.into()),
        ("max_events".into(), snapshot.max_events.into()),
        ("max_bytes".into(), snapshot.max_bytes.into()),
        ("truncated".into(), snapshot.truncated.into()),
        ("dropped_events".into(), snapshot.dropped_events.into()),
        ("events".into(), rmpv::Value::Array(events)),
    ]))
}

async fn dispatch_export_mobile_diagnostics(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    let export = daemon.export_mobile_diagnostics().await.map_err(typed_ipc_error)?;
    mobile_diagnostic_export_payload(export)
}

fn mobile_diagnostic_export_payload(
    export: styrene_ipc::types::MobileDiagnosticExport,
) -> Result<Payload, String> {
    let actual_bytes = u64::try_from(export.bytes.len())
        .map_err(|_| "mobile diagnostic export byte count exceeds u64 range")?;
    if actual_bytes > styrene_ipc::types::MOBILE_DIAGNOSTIC_MAX_BYTES {
        return Err(format!(
            "mobile diagnostic export is {actual_bytes} bytes; maximum is {}",
            styrene_ipc::types::MOBILE_DIAGNOSTIC_MAX_BYTES
        ));
    }
    if export.byte_count != actual_bytes {
        return Err("mobile diagnostic export byte_count does not match bytes".into());
    }
    ok_payload(Payload::from([
        ("schema_version".into(), export.schema_version.into()),
        ("backend_revision".into(), export.backend_revision.into()),
        ("content_type".into(), export.content_type.into()),
        ("digest_sha256".into(), export.digest_sha256.into()),
        ("first_sequence".into(), optional_value(export.first_sequence)),
        ("last_sequence".into(), optional_value(export.last_sequence)),
        ("event_count".into(), export.event_count.into()),
        ("byte_count".into(), export.byte_count.into()),
        ("max_events".into(), export.max_events.into()),
        ("max_bytes".into(), export.max_bytes.into()),
        ("truncated".into(), export.truncated.into()),
        ("dropped_events".into(), export.dropped_events.into()),
        ("bytes".into(), rmpv::Value::Binary(export.bytes)),
    ]))
}

#[cfg(test)]
mod mobile_diagnostic_projection_tests {
    use super::*;

    #[test]
    fn export_rejects_bytes_over_the_public_limit() {
        let bytes = vec![0; styrene_ipc::types::MOBILE_DIAGNOSTIC_MAX_BYTES as usize + 1];
        let export = styrene_ipc::types::MobileDiagnosticExport {
            schema_version: styrene_ipc::types::MOBILE_DIAGNOSTIC_SCHEMA_VERSION,
            backend_revision: "test".into(),
            content_type: "application/json".into(),
            digest_sha256: String::new(),
            first_sequence: None,
            last_sequence: None,
            event_count: 0,
            byte_count: bytes.len() as u64,
            max_events: styrene_ipc::types::MOBILE_DIAGNOSTIC_MAX_EVENTS,
            max_bytes: styrene_ipc::types::MOBILE_DIAGNOSTIC_MAX_BYTES,
            truncated: false,
            dropped_events: 0,
            bytes,
        };

        assert!(mobile_diagnostic_export_payload(export).unwrap_err().contains("maximum"));
    }
}

fn mobile_diagnostic_event_value(event: &styrene_ipc::types::MobileDiagnosticEvent) -> rmpv::Value {
    use styrene_ipc::types::{
        MobileDiagnosticSeverity as Severity, MobileDiagnosticSource as Source,
        MobileDiagnosticStage as Stage,
    };

    let source = match event.source {
        Source::Runtime => "runtime",
        Source::Transport => "transport",
        Source::Messaging => "messaging",
        Source::Storage => "storage",
        Source::Platform => "platform",
    };
    let stage = match event.stage {
        Stage::Boot => "boot",
        Stage::Lifecycle => "lifecycle",
        Stage::Inbound => "inbound",
        Stage::Outbound => "outbound",
        Stage::Synchronization => "synchronization",
        Stage::Persistence => "persistence",
    };
    let severity = match event.severity {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    };
    string_map([
        ("sequence", event.sequence.into()),
        ("unix_time_ms", optional_value(event.unix_time_ms)),
        ("source", source.into()),
        ("stage", stage.into()),
        ("severity", severity.into()),
        ("generation", event.generation.into()),
        ("safe_correlation", optional_value(event.safe_correlation.as_deref())),
    ])
}

async fn dispatch_query_propagation(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let mut query = styrene_ipc::types::PropagationQuery::default();
    query.cursor = val_str(payload, "cursor").map(ToOwned::to_owned);
    query.limit = payload
        .get("limit")
        .and_then(rmpv::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(100);
    let snapshot = daemon.propagation_snapshot(query).await.map_err(|error| error.to_string())?;
    let queue = snapshot
        .queue
        .iter()
        .map(|entry| {
            let mut values = vec![
                (rmpv::Value::from("id"), rmpv::Value::from(entry.id.as_str())),
                (
                    rmpv::Value::from("destination_hash"),
                    rmpv::Value::from(entry.destination_hash.as_str()),
                ),
                (rmpv::Value::from("received_at"), rmpv::Value::from(entry.received_at)),
                (rmpv::Value::from("expires_at"), rmpv::Value::from(entry.expires_at)),
                (rmpv::Value::from("size_bytes"), rmpv::Value::from(entry.size_bytes)),
                (rmpv::Value::from("state"), rmpv::Value::from(entry.state.as_str())),
            ];
            if let Some(source) = &entry.source_hash {
                values.push((rmpv::Value::from("source_hash"), rmpv::Value::from(source.as_str())));
            }
            if let Some(attempts) = entry.attempts {
                values.push((rmpv::Value::from("attempts"), rmpv::Value::from(attempts)));
            }
            rmpv::Value::Map(values)
        })
        .collect();
    let mut payload = Payload::new();
    payload.insert("enabled".into(), rmpv::Value::from(snapshot.enabled));
    payload.insert("queue_count".into(), rmpv::Value::from(snapshot.queue_count));
    payload.insert("queue_size_bytes".into(), rmpv::Value::from(snapshot.queue_size_bytes));
    payload.insert("expiry_secs".into(), rmpv::Value::from(snapshot.expiry_secs));
    payload.insert("queue".into(), rmpv::Value::Array(queue));
    payload.insert("peer_state_supported".into(), rmpv::Value::from(snapshot.peer_state_supported));
    payload.insert("sync_state_supported".into(), rmpv::Value::from(snapshot.sync_state_supported));
    if let Some(capacity) = snapshot.capacity_bytes {
        payload.insert("capacity_bytes".into(), rmpv::Value::from(capacity));
    }
    payload.insert("peers".into(), rmpv::Value::Array(Vec::new()));
    payload.insert("failures".into(), rmpv::Value::Array(Vec::new()));
    if let Some(cursor) = snapshot.next_cursor {
        payload.insert("next_cursor".into(), rmpv::Value::from(cursor));
    }
    ok_payload(payload)
}

fn standard_direction_name(
    value: styrene_ipc::types::StandardPropagationDirection,
) -> &'static str {
    use styrene_ipc::types::StandardPropagationDirection as Direction;
    match value {
        Direction::Ingress => "ingress",
        Direction::Egress => "egress",
        Direction::Sync => "sync",
        _ => "unknown",
    }
}

fn standard_stage_name(value: styrene_ipc::types::StandardPropagationStage) -> &'static str {
    use styrene_ipc::types::StandardPropagationStage as Stage;
    match value {
        Stage::Offer => "offer",
        Stage::Transfer => "transfer",
        Stage::Get => "get",
        Stage::Fetch => "fetch",
        Stage::Download => "download",
        Stage::Sync => "sync",
        Stage::Complete => "complete",
        _ => "unknown",
    }
}

fn standard_state_name(value: styrene_ipc::types::StandardPropagationAttemptState) -> &'static str {
    use styrene_ipc::types::StandardPropagationAttemptState as State;
    match value {
        State::Running => "running",
        State::Completed => "completed",
        State::Failed => "failed",
        State::Interrupted => "interrupted",
        _ => "unknown",
    }
}

fn standard_outcome_name(value: styrene_ipc::types::StandardPropagationOutcome) -> &'static str {
    use styrene_ipc::types::StandardPropagationOutcome as Outcome;
    match value {
        Outcome::Pending => "pending",
        Outcome::Completed => "completed",
        Outcome::Failed => "failed",
        Outcome::Interrupted => "interrupted",
        Outcome::CapacityRejected => "capacity_rejected",
        _ => "unknown",
    }
}

fn optional_value(value: Option<impl Into<rmpv::Value>>) -> rmpv::Value {
    value.map(Into::into).unwrap_or(rmpv::Value::Nil)
}

fn string_map(entries: impl IntoIterator<Item = (&'static str, rmpv::Value)>) -> rmpv::Value {
    rmpv::Value::Map(
        entries.into_iter().map(|(key, value)| (rmpv::Value::from(key), value)).collect(),
    )
}

async fn dispatch_query_standard_propagation(
    daemon: &Arc<dyn Daemon>,
    connection_generation: u64,
) -> Result<Payload, String> {
    let mut snapshot = daemon.query_standard_propagation().await.map_err(typed_ipc_error)?;
    if connection_generation != 0 {
        snapshot.connection_generation = Some(connection_generation);
    }
    let policy = snapshot.policy.map(|policy| {
        string_map([
            ("target_cost", policy.target_cost.into()),
            ("flexibility", policy.flexibility.into()),
            ("peering_cost", policy.peering_cost.into()),
            ("transfer_limit_kb", policy.transfer_limit_kb.into()),
            ("sync_limit_kb", policy.sync_limit_kb.into()),
            ("queue_max_count", policy.queue_max_count.into()),
            ("queue_max_bytes", policy.queue_max_bytes.into()),
            ("expiry_secs", policy.expiry_secs.into()),
            ("throttle_secs", policy.throttle_secs.into()),
            ("max_offer_links", policy.max_offer_links.into()),
        ])
    });
    let selection = snapshot.selection.map(|selection| {
        string_map([
            ("peer_hash", optional_value(selection.peer_hash)),
            ("mode", selection.mode.into()),
            ("selected_at", selection.selected_at.into()),
        ])
    });
    let peers = snapshot
        .peers
        .into_iter()
        .map(|peer| {
            string_map([
                ("peer_hash", peer.peer_hash.into()),
                ("propagation_destination_hash", optional_value(peer.propagation_destination_hash)),
                ("configured", peer.configured.into()),
                ("enabled", peer.enabled.into()),
                ("first_seen_at", peer.first_seen_at.into()),
                ("last_seen_at", peer.last_seen_at.into()),
                ("retry_at", optional_value(peer.retry_at)),
                ("backoff_count", peer.backoff_count.into()),
                ("offered_count", peer.offered_count.into()),
                ("wanted_count", peer.wanted_count.into()),
                ("accepted_count", peer.accepted_count.into()),
                ("accepted_bytes", peer.accepted_bytes.into()),
                ("failure_count", peer.failure_count.into()),
                ("transfer_limit_kb", optional_value(peer.transfer_limit_kb)),
                ("sync_limit_kb", optional_value(peer.sync_limit_kb)),
                ("stamp_cost", optional_value(peer.stamp_cost)),
                ("stamp_flexibility", optional_value(peer.stamp_flexibility)),
                ("peering_cost", optional_value(peer.peering_cost)),
            ])
        })
        .collect();
    let attempts = snapshot
        .attempts
        .into_iter()
        .map(|attempt| {
            string_map([
                ("attempt_id", attempt.attempt_id.into()),
                ("correlation_id", attempt.correlation_id.into()),
                ("peer_hash", optional_value(attempt.peer_hash)),
                ("direction", standard_direction_name(attempt.direction).into()),
                ("stage", standard_stage_name(attempt.stage).into()),
                ("state", standard_state_name(attempt.state).into()),
                ("outcome", standard_outcome_name(attempt.outcome).into()),
                ("started_at", attempt.started_at.into()),
                ("updated_at", attempt.updated_at.into()),
                ("deadline_at", optional_value(attempt.deadline_at)),
                ("offered_count", attempt.offered_count.into()),
                ("wanted_count", attempt.wanted_count.into()),
                ("accepted_count", attempt.accepted_count.into()),
                ("accepted_bytes", attempt.accepted_bytes.into()),
                ("failure_code", optional_value(attempt.failure_code)),
            ])
        })
        .collect();
    let checkpoints = snapshot
        .checkpoints
        .into_iter()
        .map(|checkpoint| {
            string_map([
                ("peer_hash", checkpoint.peer_hash.into()),
                ("direction", standard_direction_name(checkpoint.direction).into()),
                ("completed_stage", standard_stage_name(checkpoint.completed_stage).into()),
                ("item_count", checkpoint.item_count.into()),
                ("byte_count", checkpoint.byte_count.into()),
                ("last_attempt_id", optional_value(checkpoint.last_attempt_id)),
                ("updated_at", checkpoint.updated_at.into()),
            ])
        })
        .collect();
    let failures = snapshot
        .failures
        .into_iter()
        .map(|failure| {
            string_map([
                ("code", failure.code.into()),
                ("occurred_at", failure.occurred_at.into()),
                ("peer_hash", optional_value(failure.peer_hash)),
                ("attempt_id", optional_value(failure.attempt_id)),
            ])
        })
        .collect();
    let queue = string_map([
        ("queued_count", snapshot.queue.queued_count.into()),
        ("queued_bytes", snapshot.queue.queued_bytes.into()),
        ("acknowledged_count", snapshot.queue.acknowledged_count.into()),
        ("expired_count", snapshot.queue.expired_count.into()),
        ("terminal_count", snapshot.queue.terminal_count.into()),
    ]);
    ok_payload(Payload::from([
        ("version".into(), snapshot.version.into()),
        ("registered".into(), snapshot.registered.into()),
        ("active".into(), snapshot.active.into()),
        ("observed_at".into(), optional_value(snapshot.observed_at)),
        ("connection_generation".into(), optional_value(snapshot.connection_generation)),
        ("policy".into(), policy.unwrap_or(rmpv::Value::Nil)),
        ("queue".into(), queue),
        ("selection".into(), selection.unwrap_or(rmpv::Value::Nil)),
        ("peers".into(), rmpv::Value::Array(peers)),
        ("attempts".into(), rmpv::Value::Array(attempts)),
        ("checkpoints".into(), rmpv::Value::Array(checkpoints)),
        ("failures".into(), rmpv::Value::Array(failures)),
        ("peers_truncated".into(), snapshot.peers_truncated.into()),
        ("attempts_truncated".into(), snapshot.attempts_truncated.into()),
        ("checkpoints_truncated".into(), snapshot.checkpoints_truncated.into()),
        ("failures_truncated".into(), snapshot.failures_truncated.into()),
    ]))
}

// ── Identity ──────────────────────────────────────────────────────────────

async fn dispatch_query_identity(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    let info = daemon.query_identity().await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("identity_hash".into(), rmpv::Value::from(info.identity_hash.as_str()));
    p.insert("destination_hash".into(), rmpv::Value::from(info.destination_hash.as_str()));
    p.insert(
        "lxmf_destination_hash".into(),
        rmpv::Value::from(info.lxmf_destination_hash.as_str()),
    );
    p.insert("display_name".into(), rmpv::Value::from(info.display_name.as_str()));
    if let Some(ref icon) = info.icon {
        p.insert("icon".into(), rmpv::Value::from(icon.as_str()));
    }
    if let Some(ref sn) = info.short_name {
        p.insert("short_name".into(), rmpv::Value::from(sn.as_str()));
    }
    if let Some(custody) = info.custody.as_ref() {
        p.insert("custody".into(), identity_custody_value(custody));
    }
    ok_payload(p)
}

fn identity_custody_value(info: &styrene_ipc::types::IdentityCustodyInfo) -> rmpv::Value {
    use styrene_ipc::types::{
        IdentityCustodyAuthentication as Authentication,
        IdentityCustodyAvailability as Availability, IdentityCustodyBackend as Backend,
        IdentityCustodyDowngrade as Downgrade, IdentityCustodyFailureCode as FailureCode,
        IdentityCustodyProtection as Protection,
    };

    let backend = |backend| match backend {
        Backend::Keychain => "keychain",
        Backend::AndroidKeystore => "android_keystore",
        Backend::EncryptedFile => "encrypted_file",
        Backend::PlaintextFile => "plaintext_file",
    };
    let protection = |protection| match protection {
        Protection::PlatformProtected => "platform_protected",
        Protection::EncryptedAtRest => "encrypted_at_rest",
        Protection::DevelopmentPlaintext => "development_plaintext",
    };
    let authentication = match info.authentication {
        Authentication::DeviceAuthentication => "device_authentication",
        Authentication::HostKeyMaterial => "host_key_material",
        Authentication::None => "none",
    };
    let availability = match info.availability {
        Availability::Available => "available",
        Availability::Unavailable => "unavailable",
    };
    let downgrade = match info.downgrade {
        Downgrade::None => "none",
        Downgrade::ActiveBackendMismatch => "active_backend_mismatch",
    };
    let failure = info.failure.as_ref().map_or(rmpv::Value::Nil, |failure| {
        let code = match failure.code {
            FailureCode::UnsupportedTarget => "unsupported_target",
            FailureCode::FeatureDisabled => "feature_disabled",
            FailureCode::AuthenticationRequired => "authentication_required",
            FailureCode::KeyMaterialRequired => "key_material_required",
            FailureCode::BackendFailure => "backend_failure",
        };
        string_map([("code", code.into()), ("retryable", failure.retryable.into())])
    });
    string_map([
        ("requested_backend", backend(info.requested_backend).into()),
        (
            "active_backend",
            info.active_backend.map_or(rmpv::Value::Nil, |value| backend(value).into()),
        ),
        ("protection", info.protection.map_or(rmpv::Value::Nil, |value| protection(value).into())),
        ("authentication", authentication.into()),
        ("availability", availability.into()),
        ("downgrade", downgrade.into()),
        ("failure", failure),
    ])
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
            let mut fields = vec![
                (
                    rmpv::Value::from("destination_hash"),
                    rmpv::Value::from(d.destination_hash.as_str()),
                ),
                (rmpv::Value::from("identity_hash"), rmpv::Value::from(d.identity_hash.as_str())),
                (rmpv::Value::from("name"), rmpv::Value::from(d.name.as_str())),
                (rmpv::Value::from("device_type"), rmpv::Value::from(d.device_type.as_str())),
                (rmpv::Value::from("status"), rmpv::Value::from(d.status.as_str())),
                (rmpv::Value::from("is_styrene_node"), rmpv::Value::from(d.is_styrene_node)),
                (
                    rmpv::Value::from("discovered_capabilities"),
                    rmpv::Value::Array(
                        d.discovered_capabilities
                            .iter()
                            .map(|capability| rmpv::Value::from(capability.as_str()))
                            .collect(),
                    ),
                ),
            ];
            if let Some(active) = d.standard_lxmf_propagation_active {
                fields.push((
                    rmpv::Value::from("standard_lxmf_propagation_active"),
                    rmpv::Value::from(active),
                ));
            }
            rmpv::Value::Map(fields)
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

async fn dispatch_start_conversation(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hash = required_peer_hash(payload, "peer_hash")?;
    let outcome = daemon.start_conversation(peer_hash).await.map_err(typed_ipc_error)?;
    let mut response = Payload::new();
    add_outcome(&mut response, &outcome)?;
    ok_payload(response)
}

async fn dispatch_query_conversations(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let unread_only = conversation_unread_only(payload)?;
    let paged = payload.contains_key("limit") || payload.contains_key("cursor");
    let (convos, next_cursor) = if paged {
        let limit = page_limit(payload)?;
        let cursor = page_cursor(payload, "cursor")?;
        let page = daemon
            .query_conversation_page(unread_only, limit, cursor)
            .await
            .map_err(|e| e.to_string())?;
        (page.conversations, page.next_cursor)
    } else {
        (daemon.query_conversations(unread_only).await.map_err(|e| e.to_string())?, None)
    };
    let list = convos.iter().map(conversation_info_value).collect();
    let mut p = Payload::new();
    p.insert("conversations".into(), rmpv::Value::Array(list));
    if paged {
        p.insert("next_cursor".into(), next_cursor.map_or(rmpv::Value::Nil, rmpv::Value::from));
    }
    ok_payload(p)
}

fn conversation_unread_only(payload: &Payload) -> Result<bool, String> {
    let canonical = optional_bool(payload, "unread_only")?;
    // `include_unread` was the original unbounded-client spelling. Despite its
    // name, its established meaning was identical to `unread_only`.
    let legacy = optional_bool(payload, "include_unread")?;
    if canonical.is_some() && legacy.is_some() && canonical != legacy {
        return Err("unread_only and include_unread must not conflict".into());
    }
    Ok(canonical.or(legacy).unwrap_or(false))
}

fn optional_bool(payload: &Payload, key: &str) -> Result<Option<bool>, String> {
    match payload.get(key) {
        None => Ok(None),
        Some(value) => value.as_bool().map(Some).ok_or_else(|| format!("{key} must be a boolean")),
    }
}

fn conversation_info_value(c: &styrene_ipc::types::ConversationInfo) -> rmpv::Value {
    let mut fields = vec![
        (rmpv::Value::from("peer_hash"), rmpv::Value::from(c.peer_hash.as_str())),
        (rmpv::Value::from("unread_count"), rmpv::Value::from(c.unread_count)),
        (rmpv::Value::from("message_count"), rmpv::Value::from(c.message_count)),
        (rmpv::Value::from("pinned"), rmpv::Value::from(c.pinned)),
        (rmpv::Value::from("muted"), rmpv::Value::from(c.muted)),
    ];
    if let Some(name) = c.peer_name.as_deref() {
        fields.push((rmpv::Value::from("peer_name"), rmpv::Value::from(name)));
    }
    if let Some(timestamp) = c.last_message_timestamp {
        fields.push((rmpv::Value::from("last_message_timestamp"), rmpv::Value::from(timestamp)));
    }
    if let Some(content) = c.last_message_content.as_deref() {
        fields.push((rmpv::Value::from("last_message_content"), rmpv::Value::from(content)));
    }
    rmpv::Value::Map(fields)
}

// ── Messages ────────────────────────────────────────────────────────────

pub(crate) fn canonical_message_fields(
    message: &styrene_ipc::types::MessageInfo,
) -> Vec<(String, rmpv::Value)> {
    use styrene_ipc::types::{MessageAuthenticationState, MessageStampState};

    let authentication_state = match message.authentication_state {
        MessageAuthenticationState::Verified => "verified",
        MessageAuthenticationState::Invalid => "invalid",
        MessageAuthenticationState::UnknownIdentity => "unknown_identity",
        MessageAuthenticationState::NotApplicable => "not_applicable",
        MessageAuthenticationState::Unknown => "unknown",
        _ => "unknown",
    };
    let stamp_state = match message.stamp_state {
        MessageStampState::Verified => "verified",
        MessageStampState::Invalid => "invalid",
        MessageStampState::NotApplicable => "not_applicable",
        MessageStampState::Unknown => "unknown",
        _ => "unknown",
    };
    vec![
        (
            "lxmf_timestamp".into(),
            message.lxmf_timestamp.map_or(rmpv::Value::Nil, rmpv::Value::F64),
        ),
        ("authentication_state".into(), rmpv::Value::from(authentication_state)),
        ("stamp_state".into(), rmpv::Value::from(stamp_state)),
        ("stamp_value".into(), message.stamp_value.map_or(rmpv::Value::Nil, rmpv::Value::from)),
        ("stamp_cost".into(), message.stamp_cost.map_or(rmpv::Value::Nil, rmpv::Value::from)),
    ]
}

pub(crate) fn message_info_value(message: &styrene_ipc::types::MessageInfo) -> rmpv::Value {
    let mut values = vec![
        (rmpv::Value::from("id"), rmpv::Value::from(message.id.as_str())),
        (rmpv::Value::from("source_hash"), rmpv::Value::from(message.source_hash.as_str())),
        (
            rmpv::Value::from("destination_hash"),
            rmpv::Value::from(message.destination_hash.as_str()),
        ),
        (rmpv::Value::from("content"), rmpv::Value::from(message.content.as_str())),
        (
            rmpv::Value::from("title"),
            message.title.as_deref().map_or(rmpv::Value::Nil, rmpv::Value::from),
        ),
        (rmpv::Value::from("timestamp"), rmpv::Value::from(message.timestamp)),
        (rmpv::Value::from("is_outgoing"), rmpv::Value::from(message.is_outgoing)),
        (rmpv::Value::from("read"), rmpv::Value::from(message.read)),
        (rmpv::Value::from("status"), rmpv::Value::from(message.status.as_str())),
        (rmpv::Value::from("projection_complete"), rmpv::Value::from(message.projection_complete)),
        (
            rmpv::Value::from("lifecycle_state"),
            rmpv::Value::from(message_lifecycle_state_name(message.lifecycle_state)),
        ),
        (
            rmpv::Value::from("terminal_detail"),
            message.terminal_detail.as_deref().map_or(rmpv::Value::Nil, rmpv::Value::from),
        ),
    ];
    for (key, value) in [
        ("delivery_method", message.delivery_method.as_deref()),
        ("requested_delivery_method", message.requested_delivery_method.as_deref()),
        ("actual_delivery_method", message.actual_delivery_method.as_deref()),
        ("fallback_reason", message.fallback_reason.as_deref()),
        ("correlation_id", message.correlation_id.as_deref()),
    ] {
        if let Some(value) = value {
            values.push((rmpv::Value::from(key), rmpv::Value::from(value)));
        }
    }
    values.push((
        rmpv::Value::from("delivery_evidence"),
        rmpv::Value::Array(message.delivery_evidence.iter().map(delivery_evidence_value).collect()),
    ));
    values.push((
        rmpv::Value::from("attempts"),
        rmpv::Value::Array(
            message
                .attempts
                .iter()
                .map(|attempt| {
                    rmpv::Value::Map(vec![
                        (
                            rmpv::Value::from("message_id"),
                            rmpv::Value::from(attempt.message_id.as_str()),
                        ),
                        (rmpv::Value::from("number"), rmpv::Value::from(attempt.number)),
                        (
                            rmpv::Value::from("started_unix_ms"),
                            rmpv::Value::from(attempt.started_unix_ms),
                        ),
                        (
                            rmpv::Value::from("deadline_unix_ms"),
                            rmpv::Value::from(attempt.deadline_unix_ms),
                        ),
                        (rmpv::Value::from("state"), rmpv::Value::from(attempt.state.as_str())),
                        (
                            rmpv::Value::from("bearer"),
                            attempt.bearer.as_deref().map_or(rmpv::Value::Nil, rmpv::Value::from),
                        ),
                        (rmpv::Value::from("route"), message_attempt_route_value(&attempt.route)),
                    ])
                })
                .collect(),
        ),
    ));
    values.push((
        rmpv::Value::from("propagation_correlations"),
        rmpv::Value::Array(
            message
                .propagation_correlations
                .iter()
                .map(|correlation| {
                    let mut fields = vec![
                        (
                            rmpv::Value::from("relation"),
                            rmpv::Value::from(correlation.relation.as_str()),
                        ),
                        (
                            rmpv::Value::from("transient_id"),
                            rmpv::Value::from(correlation.transient_id.as_str()),
                        ),
                        (rmpv::Value::from("state"), rmpv::Value::from(correlation.state.as_str())),
                        (
                            rmpv::Value::from("created_at"),
                            rmpv::Value::from(correlation.created_at),
                        ),
                        (
                            rmpv::Value::from("updated_at"),
                            rmpv::Value::from(correlation.updated_at),
                        ),
                    ];
                    if let Some(attempt_id) = &correlation.attempt_id {
                        fields.push((
                            rmpv::Value::from("attempt_id"),
                            rmpv::Value::from(attempt_id.as_str()),
                        ));
                    }
                    if let Some(peer_hash) = &correlation.peer_hash {
                        fields.push((
                            rmpv::Value::from("peer_hash"),
                            rmpv::Value::from(peer_hash.as_str()),
                        ));
                    }
                    rmpv::Value::Map(fields)
                })
                .collect(),
        ),
    ));
    if let Some(attachment) = &message.attachment_info {
        values.push((rmpv::Value::from("attachment_info"), attachment_info_value(attachment)));
    }
    values.push((
        rmpv::Value::from("attachments"),
        rmpv::Value::Array(message.attachments.iter().map(attachment_info_value).collect()),
    ));
    values.extend(
        canonical_message_fields(message)
            .into_iter()
            .map(|(key, value)| (rmpv::Value::from(key), value)),
    );
    rmpv::Value::Map(values)
}

fn message_attempt_route_value(
    route: &styrene_ipc::types::MessageAttemptRouteObservation,
) -> rmpv::Value {
    use styrene_ipc::types::MessageAttemptRouteOutcome;

    let outcome = match route.outcome {
        MessageAttemptRouteOutcome::Observed => "observed",
        MessageAttemptRouteOutcome::Unknown => "unknown",
        _ => "unknown",
    };
    let interface = route.interface.as_ref().map_or(rmpv::Value::Nil, |interface| {
        string_map([
            ("id", interface.id.as_str().into()),
            ("kind", interface.kind.as_str().into()),
            ("generation", interface.generation.into()),
        ])
    });
    string_map([
        ("outcome", outcome.into()),
        ("connection_generation", optional_value(route.connection_generation)),
        ("observed_at", optional_value(route.observed_at)),
        ("next_hop", optional_value(route.next_hop.as_deref())),
        ("hops", optional_value(route.hops)),
        ("stale", route.stale.into()),
        ("interface", interface),
    ])
}

#[cfg(test)]
mod message_attempt_projection_tests {
    use super::*;

    #[test]
    fn default_route_is_projected_as_explicit_unknown() {
        let route = styrene_ipc::types::MessageAttemptRouteObservation::default();
        let rmpv::Value::Map(fields) = message_attempt_route_value(&route) else {
            panic!("route projection must be a map");
        };
        let field = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key.as_str() == Some(name))
                .map(|(_, value)| value)
                .expect("route field")
        };
        assert_eq!(field("outcome").as_str(), Some("unknown"));
        assert!(field("connection_generation").is_nil());
        assert!(field("observed_at").is_nil());
        assert!(field("next_hop").is_nil());
        assert!(field("hops").is_nil());
        assert!(!field("stale").as_bool().expect("stale boolean"));
        assert!(field("interface").is_nil());
    }
}

fn attachment_info_value(attachment: &styrene_ipc::types::AttachmentInfo) -> rmpv::Value {
    let mut values = vec![
        (rmpv::Value::from("ordinal"), rmpv::Value::from(attachment.ordinal)),
        (rmpv::Value::from("id"), rmpv::Value::from(attachment.id.as_str())),
        (rmpv::Value::from("name"), rmpv::Value::from(attachment.name.as_str())),
        (rmpv::Value::from("content_type"), rmpv::Value::from(attachment.content_type.as_str())),
        (rmpv::Value::from("size"), rmpv::Value::from(attachment.size)),
        (rmpv::Value::from("checksum"), rmpv::Value::from(attachment.checksum.as_str())),
        (rmpv::Value::from("availability"), rmpv::Value::from(attachment.availability.as_str())),
        (rmpv::Value::from("integrity"), rmpv::Value::from(attachment.integrity.as_str())),
    ];
    if let Some(transfer) = &attachment.transfer {
        values.push((
            rmpv::Value::from("transfer"),
            rmpv::Value::Map(vec![
                (rmpv::Value::from("message_id"), rmpv::Value::from(transfer.message_id.as_str())),
                (
                    rmpv::Value::from("resource_hash"),
                    transfer.resource_hash.as_deref().map_or(rmpv::Value::Nil, rmpv::Value::from),
                ),
                (
                    rmpv::Value::from("transfer_id"),
                    rmpv::Value::from(transfer.transfer_id.as_str()),
                ),
                (
                    rmpv::Value::from("representation"),
                    rmpv::Value::from(transfer.representation.as_str()),
                ),
                (rmpv::Value::from("direction"), rmpv::Value::from(transfer.direction.as_str())),
                (rmpv::Value::from("state"), rmpv::Value::from(transfer.state.as_str())),
                (rmpv::Value::from("transferred"), rmpv::Value::from(transfer.transferred)),
                (rmpv::Value::from("total"), rmpv::Value::from(transfer.total)),
                (
                    rmpv::Value::from("checksum_verified"),
                    rmpv::Value::from(transfer.checksum_verified),
                ),
                (rmpv::Value::from("cancellable"), rmpv::Value::from(transfer.cancellable)),
                (
                    rmpv::Value::from("error"),
                    transfer.error.as_deref().map_or(rmpv::Value::Nil, rmpv::Value::from),
                ),
            ]),
        ));
    }
    rmpv::Value::Map(values)
}

fn message_lifecycle_state_name(state: styrene_ipc::types::MessageLifecycleState) -> &'static str {
    use styrene_ipc::types::MessageLifecycleState::*;
    match state {
        Queued => "queued",
        Sending => "sending",
        Sent => "sent",
        Delivered => "delivered",
        Failed => "failed",
        Cancelled => "cancelled",
        Expired => "expired",
        Rejected => "rejected",
        Unknown => "unknown",
        _ => "unknown",
    }
}

fn delivery_evidence_value(
    evidence: &styrene_ipc::types::MessageDeliveryEvidenceInfo,
) -> rmpv::Value {
    use styrene_ipc::types::{
        MessageDeliveryEvidenceKind as Kind, MessageDeliveryEvidenceState as State,
    };
    let kind = match evidence.kind {
        Kind::PacketReceipt => "packet_receipt",
        Kind::ResourceCompletion => "resource_completion",
        _ => "unknown",
    };
    let state = match evidence.state {
        State::Tracked => "tracked",
        State::Completed => "completed",
        State::Failed => "failed",
        State::Cancelled => "cancelled",
        _ => "unknown",
    };
    rmpv::Value::Map(vec![
        (rmpv::Value::from("kind"), rmpv::Value::from(kind)),
        (rmpv::Value::from("hash"), rmpv::Value::from(evidence.hash.as_str())),
        (rmpv::Value::from("representation"), rmpv::Value::from(evidence.representation.as_str())),
        (rmpv::Value::from("state"), rmpv::Value::from(state)),
        (
            rmpv::Value::from("outcome"),
            evidence.outcome.as_deref().map_or(rmpv::Value::Nil, rmpv::Value::from),
        ),
        (
            rmpv::Value::from("attempt"),
            evidence.attempt.map_or(rmpv::Value::Nil, rmpv::Value::from),
        ),
        (
            rmpv::Value::from("correlation_id"),
            evidence.correlation_id.as_deref().map_or(rmpv::Value::Nil, rmpv::Value::from),
        ),
        (rmpv::Value::from("observed_at"), rmpv::Value::from(evidence.observed_at)),
        (
            rmpv::Value::from("terminal_at"),
            evidence.terminal_at.map_or(rmpv::Value::Nil, rmpv::Value::from),
        ),
        (
            rmpv::Value::from("transferred_bytes"),
            evidence.transferred_bytes.map_or(rmpv::Value::Nil, rmpv::Value::from),
        ),
        (
            rmpv::Value::from("total_bytes"),
            evidence.total_bytes.map_or(rmpv::Value::Nil, rmpv::Value::from),
        ),
        (
            rmpv::Value::from("progress"),
            evidence.progress.map_or(rmpv::Value::Nil, rmpv::Value::from),
        ),
    ])
}

async fn dispatch_query_attachment(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let message_id = val_str(payload, "message_id").ok_or("missing message_id")?;
    let chunked = payload.contains_key("ordinal")
        || payload.contains_key("offset")
        || payload.contains_key("max_bytes");
    let (attachment, data, next_offset, done) = if chunked {
        let ordinal = payload.get("ordinal").and_then(rmpv::Value::as_u64).unwrap_or(0);
        let ordinal = u8::try_from(ordinal).map_err(|_| "ordinal must be between 0 and 7")?;
        let offset = payload.get("offset").and_then(rmpv::Value::as_u64).unwrap_or(0);
        let max_bytes =
            payload.get("max_bytes").and_then(rmpv::Value::as_u64).unwrap_or(256 * 1024);
        let max_bytes = u32::try_from(max_bytes).map_err(|_| "max_bytes exceeds u32")?;
        let chunk = daemon
            .query_attachment_chunk(message_id, ordinal, offset, max_bytes)
            .await
            .map_err(|error| error.to_string())?;
        (chunk.attachment, chunk.data, chunk.next_offset, chunk.done)
    } else {
        let data = daemon.query_attachment(message_id).await.map_err(|error| error.to_string())?;
        let attachment = daemon
            .list_attachments(message_id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .next()
            .ok_or("attachment metadata unavailable")?;
        let len = data.len() as u64;
        (attachment, data, len, true)
    };
    let mut response = Payload::new();
    response.insert("attachment".into(), attachment_info_value(&attachment));
    response.insert("data".into(), rmpv::Value::Binary(data));
    response.insert("next_offset".into(), rmpv::Value::from(next_offset));
    response.insert("done".into(), rmpv::Value::from(done));
    ok_payload(response)
}

async fn dispatch_query_attachment_transfer(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let message_id = val_str(payload, "message_id").ok_or("missing message_id")?;
    let transfer = daemon.query_attachment_transfer(message_id).await.map_err(typed_ipc_error)?;
    let value =
        rmpv::ext::to_value(serde_json::to_value(transfer).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    ok_payload(HashMap::from([("attachment_transfer".into(), value)]))
}

async fn dispatch_cancel_attachment_transfer(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let message_id = val_str(payload, "message_id").ok_or("missing message_id")?;
    let outcome = daemon.cancel_attachment_transfer(message_id).await.map_err(typed_ipc_error)?;
    let mut response = Payload::new();
    response.insert(
        "success".into(),
        rmpv::Value::from(outcome.disposition == styrene_ipc::types::MessagingDisposition::Applied),
    );
    add_outcome(&mut response, &outcome)?;
    ok_payload(response)
}

async fn dispatch_query_messages(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hash = required_peer_hash(payload, "peer_hash")?;
    let before_ts = match payload.get("before_ts") {
        None | Some(rmpv::Value::Nil) => None,
        Some(value) => Some(
            value
                .as_i64()
                .ok_or_else(|| "before_ts must be a signed integer or nil".to_string())?,
        ),
    };
    let cursor = page_cursor(payload, "cursor")?;
    if cursor.is_some() && before_ts.is_some() {
        return Err("cursor and before_ts are mutually exclusive".into());
    }
    let limit = page_limit(payload)?;
    let (msgs, next_cursor) = if before_ts.is_some() {
        (daemon.query_messages(peer_hash, limit, before_ts).await.map_err(|e| e.to_string())?, None)
    } else {
        let page =
            daemon.query_message_page(peer_hash, limit, cursor).await.map_err(|e| e.to_string())?;
        (page.messages, page.next_cursor)
    };
    let list: Vec<rmpv::Value> = msgs.iter().map(message_info_value).collect();
    let mut p = Payload::new();
    p.insert("messages".into(), rmpv::Value::Array(list));
    if before_ts.is_none() {
        p.insert("next_cursor".into(), next_cursor.map_or(rmpv::Value::Nil, rmpv::Value::from));
    }
    ok_payload(p)
}

async fn dispatch_query_message(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let message_id = val_str(payload, "message_id").ok_or("missing message_id")?;
    if message_id.is_empty() || message_id.len() > 128 {
        return Err("message_id must contain between 1 and 128 bytes".into());
    }
    let message = daemon.query_message(message_id).await.map_err(typed_ipc_error)?;
    let mut response = Payload::new();
    response
        .insert("message".into(), message.as_ref().map_or(rmpv::Value::Nil, message_info_value));
    ok_payload(response)
}

// ── Send chat ───────────────────────────────────────────────────────────

async fn dispatch_send_chat(
    daemon: &Arc<dyn Daemon>,
    payload: Payload,
    authoritative_outcome: bool,
) -> Result<Payload, String> {
    let peer_hash = required_peer_hash(&payload, "peer_hash")?.to_string();
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
    if let Some(value) = payload.get("attachment") {
        req.attachment = Some(
            value
                .as_slice()
                .filter(|bytes| bytes.len() <= 768 * 1024)
                .ok_or("attachment must be binary and at most 768 KiB")?
                .to_vec(),
        );
        req.attachment_name = val_str(&payload, "attachment_name").map(str::to_owned);
        if payload.contains_key("attachment_name") && req.attachment_name.is_none() {
            return Err("attachment_name must be a string".into());
        }
    } else if payload.contains_key("attachment_name") {
        return Err("attachment_name requires legacy attachment".into());
    }
    if let Some(value) = payload.get("attachments") {
        if req.attachment.is_some() {
            return Err("legacy attachment and attachments are mutually exclusive".into());
        }
        let entries = value.as_array().ok_or("attachments must be an array")?;
        if entries.len() > 8 {
            return Err("attachment count exceeds 8".into());
        }
        let mut aggregate = 0usize;
        for entry in entries {
            let map = entry.as_map().ok_or("attachment entry must be a map")?;
            let get = |key: &str| {
                map.iter()
                    .find(|(candidate, _)| candidate.as_str() == Some(key))
                    .map(|(_, value)| value)
            };
            let name = get("name")
                .and_then(rmpv::Value::as_str)
                .filter(|name| (1..=255).contains(&name.len()))
                .ok_or("attachment name must be 1..=255 UTF-8 bytes")?;
            let bytes = get("bytes")
                .or_else(|| get("data"))
                .and_then(rmpv::Value::as_slice)
                .filter(|bytes| bytes.len() <= 768 * 1024)
                .ok_or("attachment bytes must be binary and at most 768 KiB")?;
            aggregate =
                aggregate.checked_add(bytes.len()).ok_or("attachment aggregate overflow")?;
            if aggregate > 768 * 1024 {
                return Err("attachment aggregate exceeds 768 KiB".into());
            }
            let mut input = styrene_ipc::types::AttachmentInput::default();
            input.name = name.to_owned();
            input.bytes = bytes.to_vec();
            input.content_type = match get("content_type") {
                None | Some(rmpv::Value::Nil) => None,
                Some(value) => Some(
                    value
                        .as_str()
                        .ok_or("attachment content_type must be a string or nil")?
                        .to_owned(),
                ),
            };
            input.expected_sha256 = match get("expected_sha256") {
                None | Some(rmpv::Value::Nil) => None,
                Some(value) => {
                    let expected = value
                        .as_str()
                        .ok_or("attachment expected_sha256 must be a string or nil")?;
                    if expected.len() != 64
                        || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
                        || expected.bytes().any(|byte| byte.is_ascii_uppercase())
                    {
                        return Err(
                            "attachment expected_sha256 must be 64 lowercase hex characters".into(),
                        );
                    }
                    Some(expected.to_owned())
                }
            };
            req.attachments.push(input);
        }
    }
    let mut p = Payload::new();
    if authoritative_outcome {
        let outcome = daemon.send_chat_outcome(req).await.map_err(typed_ipc_error)?;
        if outcome.message_id.is_empty() || outcome.message.id != outcome.message_id {
            return Err("daemon send outcome omitted its authoritative message projection".into());
        }
        p.insert("message_id".into(), rmpv::Value::from(outcome.message_id.as_str()));
        p.insert("outcome".into(), serialized_value(&outcome)?);
    } else {
        let msg_id = daemon.send_chat(req).await.map_err(|e| e.to_string())?;
        p.insert("message_id".into(), rmpv::Value::from(msg_id.as_str()));
    }
    ok_payload(p)
}

async fn dispatch_query_draft(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hash = required_peer_hash(payload, "peer_hash")?;
    let draft = daemon.draft(peer_hash).await.map_err(typed_ipc_error)?;
    Ok(Payload::from([(
        "draft".into(),
        match draft {
            Some(draft) => serialized_value(&draft)?,
            None => rmpv::Value::Nil,
        },
    )]))
}

async fn dispatch_set_draft(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hash = required_peer_hash(payload, "peer_hash")?;
    let content = required_str(payload, "content")?;
    if content.len() > styrene_ipc::types::MAX_CHAT_CONTENT_BYTES {
        return Err(invalid_dispatch("draft content exceeds 65536 UTF-8 bytes"));
    }
    let draft = daemon.set_draft(peer_hash, content).await.map_err(typed_ipc_error)?;
    Ok(Payload::from([("draft".into(), serialized_value(&draft)?)]))
}

async fn dispatch_clear_draft(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hash = required_peer_hash(payload, "peer_hash")?;
    let disposition = daemon.clear_draft(peer_hash).await.map_err(typed_ipc_error)?;
    Ok(Payload::from([("disposition".into(), serialized_value(&disposition)?)]))
}

// ── Mark read ───────────────────────────────────────────────────────────

async fn dispatch_mark_read(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hash = required_peer_hash(payload, "peer_hash")?;
    let outcome = daemon.mark_read_outcome(peer_hash).await.map_err(typed_ipc_error)?;
    let mut p = Payload::new();
    p.insert("count".into(), rmpv::Value::from(outcome.affected_count));
    add_outcome(&mut p, &outcome)?;
    ok_payload(p)
}

// ── Delete conversation ─────────────────────────────────────────────────

async fn dispatch_delete_conversation(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hash = required_peer_hash(payload, "peer_hash")?;
    let outcome = daemon.delete_conversation_outcome(peer_hash).await.map_err(typed_ipc_error)?;
    let mut p = Payload::new();
    p.insert("count".into(), rmpv::Value::from(outcome.affected_count));
    add_outcome(&mut p, &outcome)?;
    ok_payload(p)
}

// ── Delete message ──────────────────────────────────────────────────────

async fn dispatch_delete_message(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let message_id = required_message_id(payload)?;
    let outcome = daemon.delete_message_outcome(message_id).await.map_err(typed_ipc_error)?;
    let mut p = Payload::new();
    p.insert(
        "success".into(),
        rmpv::Value::from(outcome.disposition == styrene_ipc::types::MessagingDisposition::Applied),
    );
    add_outcome(&mut p, &outcome)?;
    ok_payload(p)
}

// ── Contacts ────────────────────────────────────────────────────────────

async fn dispatch_query_contacts(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    let contacts = daemon.query_contacts().await.map_err(|e| e.to_string())?;
    let list: Vec<rmpv::Value> = contacts
        .iter()
        .map(|c| {
            let m = vec![
                (rmpv::Value::from("peer_hash"), rmpv::Value::from(c.peer_hash.as_str())),
                (
                    rmpv::Value::from("alias"),
                    c.alias.as_deref().map_or(rmpv::Value::Nil, rmpv::Value::from),
                ),
                (
                    rmpv::Value::from("notes"),
                    c.notes.as_deref().map_or(rmpv::Value::Nil, rmpv::Value::from),
                ),
                (
                    rmpv::Value::from("created_at"),
                    c.created_at.map_or(rmpv::Value::Nil, rmpv::Value::from),
                ),
                (
                    rmpv::Value::from("updated_at"),
                    c.updated_at.map_or(rmpv::Value::Nil, rmpv::Value::from),
                ),
            ];
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
    let result = daemon.resolve_name(name, prefix).await.map_err(|e| e.to_string())?;
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
    let changed =
        daemon.set_identity(display_name, icon, short_name).await.map_err(|e| e.to_string())?;
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
    let changed =
        daemon.set_auto_reply(mode, message, cooldown).await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("changed".into(), rmpv::Value::Boolean(changed));
    ok_payload(p)
}

async fn dispatch_search_messages(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let query = val_str(payload, "query").ok_or_else(|| invalid_dispatch("missing query"))?;
    if query.is_empty() || query.len() > 1024 {
        return Err(invalid_dispatch("search query must be 1..=1024 UTF-8 bytes"));
    }
    let peer_hash = val_str(payload, "peer_hash")
        .map(|peer| validate_peer_hash(peer))
        .transpose()
        .map_err(invalid_dispatch)?;
    let limit = message_query_limit(payload).map_err(invalid_dispatch)?;
    let outcome =
        daemon.search_messages_outcome(query, peer_hash, limit).await.map_err(typed_ipc_error)?;
    let arr: Vec<rmpv::Value> = outcome.messages.iter().map(message_info_value).collect();
    let mut p = Payload::new();
    p.insert("messages".into(), rmpv::Value::Array(arr));
    p.insert("outcome".into(), serialized_value(&outcome)?);
    ok_payload(p)
}

async fn dispatch_retry_message(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let message_id = required_message_id(payload)?;
    let outcome = daemon.retry_message_outcome(message_id).await.map_err(typed_ipc_error)?;
    let mut p = Payload::new();
    p.insert(
        "retried".into(),
        rmpv::Value::Boolean(
            outcome.disposition == styrene_ipc::types::MessagingDisposition::Applied,
        ),
    );
    add_outcome(&mut p, &outcome)?;
    ok_payload(p)
}

async fn dispatch_cancel_message(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let message_id = required_message_id(payload)?;
    let outcome = daemon.cancel_message_outcome(message_id).await.map_err(typed_ipc_error)?;
    let mut payload = Payload::from([(
        "cancelled".into(),
        rmpv::Value::Boolean(
            outcome.disposition == styrene_ipc::types::MessagingDisposition::Applied,
        ),
    )]);
    add_outcome(&mut payload, &outcome)?;
    ok_payload(payload)
}

// ── Query Config ─────────────────────────────────────────────────────────────

async fn dispatch_query_config(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
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
    let peer_hash = required_peer_hash(payload, "peer_hash")?;
    let alias = val_str(payload, "alias");
    let notes = val_str(payload, "notes");
    let outcome =
        daemon.set_contact_outcome(peer_hash, alias, notes).await.map_err(typed_ipc_error)?;
    let mut p = Payload::new();
    p.insert(
        "ok".into(),
        rmpv::Value::Boolean(matches!(
            outcome.disposition,
            styrene_ipc::types::MessagingDisposition::Applied
                | styrene_ipc::types::MessagingDisposition::Created
                | styrene_ipc::types::MessagingDisposition::Updated
        )),
    );
    if let Some(contact) = outcome.contact.as_ref() {
        p.insert("contact".into(), serialized_value(contact)?);
    }
    add_outcome(&mut p, &outcome)?;
    ok_payload(p)
}

async fn dispatch_remove_contact(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hash = required_peer_hash(payload, "peer_hash")?;
    let outcome = daemon.remove_contact_outcome(peer_hash).await.map_err(typed_ipc_error)?;
    let mut p = Payload::new();
    p.insert(
        "removed".into(),
        rmpv::Value::Boolean(
            outcome.disposition == styrene_ipc::types::MessagingDisposition::Applied,
        ),
    );
    add_outcome(&mut p, &outcome)?;
    ok_payload(p)
}

async fn dispatch_conversation_flag(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    operation: &str,
) -> Result<Payload, String> {
    let peer_hash = val_str(payload, "peer_hash")
        .ok_or_else(|| invalid_dispatch("missing peer_hash"))
        .and_then(|peer| validate_peer_hash(peer).map_err(invalid_dispatch))?;
    let outcome = match operation {
        "pin" => daemon.pin_conversation_outcome(peer_hash).await,
        "unpin" => daemon.unpin_conversation_outcome(peer_hash).await,
        "mute" => daemon.mute_conversation_outcome(peer_hash).await,
        "unmute" => daemon.unmute_conversation_outcome(peer_hash).await,
        _ => return Err("invalid conversation flag operation".into()),
    }
    .map_err(typed_ipc_error)?;
    let mut response = Payload::from([(
        "success".into(),
        rmpv::Value::Boolean(
            outcome.disposition == styrene_ipc::types::MessagingDisposition::Applied,
        ),
    )]);
    add_outcome(&mut response, &outcome)?;
    ok_payload(response)
}

// ── Device Status (fleet RPC) ────────────────────────────────────────────────

async fn dispatch_device_status(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let dest = val_str(payload, "destination_hash").ok_or("missing destination_hash")?;
    let dest = validate_peer_hash(dest)?;
    let timeout = payload.get("timeout").and_then(|v| v.as_u64());
    let info = daemon.device_status(dest, timeout).await.map_err(|e| e.to_string())?;
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

async fn dispatch_sub_devices(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    let _ = daemon.subscribe_devices().await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("subscribed".into(), rmpv::Value::Boolean(true));
    ok_payload(p)
}

async fn dispatch_sub_messages(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer_hashes = payload
        .get("peer_hashes")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| invalid_dispatch("peer_hashes must be an array"))?
                .iter()
                .map(|value| {
                    let peer = value
                        .as_str()
                        .ok_or_else(|| invalid_dispatch("peer_hashes entries must be strings"))?;
                    validate_peer_hash(peer).map(str::to_string).map_err(invalid_dispatch)
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let _ = daemon.subscribe_messages(&peer_hashes).await.map_err(|e| e.to_string())?;
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

async fn dispatch_get_unread_counts(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    // Build unread counts from conversations
    let convos = daemon.query_conversations(true).await.unwrap_or_default();
    let mut counts = HashMap::new();
    for c in &convos {
        if c.unread_count > 0 {
            counts.insert(c.peer_hash.clone(), rmpv::Value::from(c.unread_count as i64));
        }
    }
    let mut p = Payload::new();
    p.insert(
        "counts".into(),
        rmpv::Value::Map(
            counts.into_iter().map(|(k, v)| (rmpv::Value::from(k.as_str()), v)).collect(),
        ),
    );
    ok_payload(p)
}

async fn dispatch_get_nodes(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    // GET_NODES returns persisted nodes — same data as QUERY_DEVICES
    let styrene_only = payload.get("styrene_only").and_then(|v| v.as_bool()).unwrap_or(false);
    let devices = daemon.query_devices(styrene_only).await.map_err(|e| e.to_string())?;
    let arr: Vec<rmpv::Value> = devices
        .iter()
        .map(|d| {
            let mut item = HashMap::new();
            item.insert(
                "destination_hash".to_string(),
                rmpv::Value::from(d.destination_hash.as_str()),
            );
            item.insert("name".to_string(), rmpv::Value::from(d.name.as_str()));
            item.insert("status".to_string(), rmpv::Value::from(d.status.as_str()));
            item.insert("is_styrene_node".to_string(), rmpv::Value::Boolean(d.is_styrene_node));
            if let Some(ts) = d.last_announce {
                item.insert("last_announce".to_string(), rmpv::Value::from(ts));
            }
            rmpv::Value::Map(
                item.into_iter().map(|(k, v)| (rmpv::Value::from(k.as_str()), v)).collect(),
            )
        })
        .collect();
    let mut p = Payload::new();
    p.insert("nodes".into(), rmpv::Value::Array(arr));
    ok_payload(p)
}

async fn dispatch_get_core_config(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
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

async fn dispatch_sub_links() -> Result<Payload, String> {
    // Acknowledge link telemetry subscription — EventLink frames pushed when
    // link status or RTT changes. The daemon emits these from the transport layer.
    let mut p = Payload::new();
    p.insert("subscribed".into(), rmpv::Value::Boolean(true));
    ok_payload(p)
}

async fn dispatch_query_links(
    daemon: &Arc<dyn Daemon>,
    connection_generation: u64,
) -> Result<Payload, String> {
    let snapshot = daemon.link_snapshot().await.map_err(|error| error.to_string())?;
    let mut payload = Payload::new();
    payload.insert(
        "active".into(),
        rmpv::Value::Array(
            snapshot
                .active
                .iter()
                .map(|event| link_event_value(event, connection_generation))
                .collect(),
        ),
    );
    payload.insert(
        "history".into(),
        rmpv::Value::Array(
            snapshot
                .history
                .iter()
                .map(|event| link_event_value(event, connection_generation))
                .collect(),
        ),
    );
    ok_payload(payload)
}

fn link_event_value(event: &styrene_ipc::types::LinkEvent, generation: u64) -> rmpv::Value {
    use styrene_ipc::types::{LinkActivity, LinkEventKind, LinkLifecycleReason};

    let kind = match event.kind {
        LinkEventKind::Established => "established",
        LinkEventKind::Identified => "identified",
        LinkEventKind::Activity => "activity",
        LinkEventKind::RttUpdated => "rtt_updated",
        LinkEventKind::Teardown => "teardown",
        LinkEventKind::Timeout => "timeout",
        _ => "unknown",
    };
    let activity = match event.activity {
        LinkActivity::Active => "active",
        LinkActivity::Historical => "historical",
        _ => "unknown",
    };
    let mut fields = vec![
        (rmpv::Value::from("link_id"), rmpv::Value::from(event.link_id.as_str())),
        (rmpv::Value::from("peer_hash"), rmpv::Value::from(event.peer_hash.as_str())),
        (rmpv::Value::from("status"), rmpv::Value::from(event.status.as_str())),
        (rmpv::Value::from("kind"), rmpv::Value::from(kind)),
        (rmpv::Value::from("activity"), rmpv::Value::from(activity)),
        (rmpv::Value::from("identified"), rmpv::Value::from(event.identified)),
        (rmpv::Value::from("timestamp"), rmpv::Value::from(event.timestamp)),
        (rmpv::Value::from("source"), rmpv::Value::from(event.observation.source.as_str())),
        (rmpv::Value::from("connection_generation"), rmpv::Value::from(generation)),
        (rmpv::Value::from("stale"), rmpv::Value::from(event.observation.stale)),
    ];
    if let Some(interface) = &event.interface {
        fields.push((rmpv::Value::from("interface"), rmpv::Value::from(interface.as_str())));
    }
    if let Some(identity) = &event.remote_identity_hash {
        fields.push((
            rmpv::Value::from("remote_identity_hash"),
            rmpv::Value::from(identity.as_str()),
        ));
    }
    if let Some(rtt_ms) = event.rtt_ms {
        fields.push((rmpv::Value::from("rtt_ms"), rmpv::Value::F64(rtt_ms)));
    }
    if let Some(observed_at) = event.observation.observed_at {
        fields.push((rmpv::Value::from("observed_at"), rmpv::Value::from(observed_at)));
    }
    if let Some(reason) = event.reason {
        let reason = match reason {
            LinkLifecycleReason::LocalTeardown => "local_teardown",
            LinkLifecycleReason::StaleTimeout => "stale_timeout",
            LinkLifecycleReason::EstablishmentTimeout => "establishment_timeout",
            LinkLifecycleReason::ChannelTimeout => "channel_timeout",
            LinkLifecycleReason::SendFailure => "send_failure",
            _ => "unknown",
        };
        fields.push((rmpv::Value::from("reason"), rmpv::Value::from(reason)));
    }
    rmpv::Value::Map(fields)
}

// ── Exec / Reboot (fleet RPC) ────────────────────────────────────────────────

async fn dispatch_exec(daemon: &Arc<dyn Daemon>, payload: &Payload) -> Result<Payload, String> {
    let dest = val_str(payload, "destination_hash").ok_or("missing destination_hash")?;
    let dest = validate_peer_hash(dest)?;
    let cmd = val_str(payload, "command").ok_or("missing command")?;
    let args: Vec<String> = payload
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let timeout = payload.get("timeout").and_then(|v| v.as_u64());
    let result = daemon.exec(dest, cmd, args, timeout).await.map_err(|e| e.to_string())?;
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
    let dest = validate_peer_hash(dest)?;
    let delay = payload.get("delay").and_then(|v| v.as_u64());
    let timeout = payload.get("timeout").and_then(|v| v.as_u64());
    let result = daemon.reboot_device(dest, delay, timeout).await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("accepted".into(), rmpv::Value::Boolean(result.accepted));
    if let Some(d) = result.delay_secs {
        p.insert("delay_secs".into(), rmpv::Value::from(d as i64));
    }
    ok_payload(p)
}

// ── Send (generic LXMF send — wraps send_chat) ──────────────────────────────

async fn dispatch_send(daemon: &Arc<dyn Daemon>, payload: &Payload) -> Result<Payload, String> {
    let peer_hash = val_str(payload, "destination_hash")
        .or_else(|| val_str(payload, "peer_hash"))
        .ok_or("missing destination_hash or peer_hash")?;
    let peer_hash = validate_peer_hash(peer_hash)?.to_string();
    let content = val_str(payload, "content").unwrap_or("").to_string();
    if content.len() > 65536 {
        return Err(format!("content too large: {} bytes", content.len()));
    }
    let title = val_str(payload, "title").map(|s| s.to_string());
    let mut req = styrene_ipc::types::SendChatRequest::default();
    req.peer_hash = peer_hash;
    req.content = content;
    req.title = title;
    let msg_id = daemon.send_chat(req).await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("message_id".into(), rmpv::Value::from(msg_id.as_str()));
    ok_payload(p)
}

// ── Peer blocking (stub — not yet in Daemon trait) ───────────────────────────

async fn dispatch_block_peer(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let hash = val_str(payload, "identity_hash").ok_or("missing identity_hash")?;
    validate_peer_hash(hash)?;
    daemon
        .block_peer(hash)
        .await
        .map(|blocked| {
            let mut p = Payload::new();
            p.insert("blocked".into(), rmpv::Value::Boolean(blocked));
            p
        })
        .map_err(|e| e.to_string())
}

async fn dispatch_unblock_peer(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let hash = val_str(payload, "identity_hash").ok_or("missing identity_hash")?;
    validate_peer_hash(hash)?;
    daemon
        .unblock_peer(hash)
        .await
        .map(|unblocked| {
            let mut p = Payload::new();
            p.insert("unblocked".into(), rmpv::Value::Boolean(unblocked));
            p
        })
        .map_err(|e| e.to_string())
}

async fn dispatch_blocked_peers(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    daemon
        .blocked_peers()
        .await
        .map(|peers| {
            let mut p = Payload::new();
            let arr: Vec<rmpv::Value> = peers.into_iter().map(rmpv::Value::from).collect();
            p.insert("blocked_peers".into(), rmpv::Value::Array(arr));
            p
        })
        .map_err(|e| e.to_string())
}

// ── Config save ─────────────────────────────────────────────────────────────

async fn dispatch_save_core_config(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    // Pass an empty ConfigSnapshot — the daemon reloads from disk
    let snapshot = styrene_ipc::types::ConfigSnapshot::default();
    daemon
        .save_config(snapshot)
        .await
        .map(|saved| {
            let mut p = Payload::new();
            p.insert("saved".into(), rmpv::Value::Boolean(saved));
            p
        })
        .map_err(|e| e.to_string())
}

// ── Sync messages (stub — PropagationClient feature) ─────────────────────────

async fn dispatch_sync_messages() -> Result<Payload, String> {
    // Honest stub: sync is a PropagationClient feature not yet built.
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

fn append_observation(
    values: &mut Vec<(rmpv::Value, rmpv::Value)>,
    observation: &styrene_ipc::types::ObservationMetadata,
    connection_generation: u64,
) {
    values.push((rmpv::Value::from("source"), rmpv::Value::from(observation.source.as_str())));
    if let Some(observed_at) = observation.observed_at {
        values.push((rmpv::Value::from("observed_at"), rmpv::Value::from(observed_at)));
    }
    if let Some(generation) = observation.connection_generation {
        values.push((rmpv::Value::from("connection_generation"), rmpv::Value::from(generation)));
    }
    let ipc_generation = (connection_generation != 0)
        .then_some(connection_generation)
        .or(observation.ipc_connection_generation);
    if let Some(generation) = ipc_generation {
        values
            .push((rmpv::Value::from("ipc_connection_generation"), rmpv::Value::from(generation)));
    }
    if let Some(generation) = observation.interface_generation {
        values.push((rmpv::Value::from("interface_generation"), rmpv::Value::from(generation)));
    }
    if let Some(age) = observation.age_secs {
        values.push((rmpv::Value::from("age_secs"), rmpv::Value::from(age)));
    }
    if let Some(threshold) = observation.freshness_threshold_secs {
        values.push((rmpv::Value::from("freshness_threshold_secs"), rmpv::Value::from(threshold)));
    }
    values.push((rmpv::Value::from("stale"), rmpv::Value::from(observation.stale)));
    if let Some(correlation_id) = &observation.correlation_id {
        values.push((
            rmpv::Value::from("correlation_id"),
            rmpv::Value::from(correlation_id.as_str()),
        ));
    }
}

async fn dispatch_query_path_info(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    connection_generation: u64,
) -> Result<Payload, String> {
    let dest = val_str(payload, "destination_hash").ok_or("missing destination_hash")?;
    let dest = validate_peer_hash(dest)?;
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
    if let Some(expires) = info.expires {
        p.insert("expires".into(), rmpv::Value::from(expires));
    }
    if let Some(next_hop) = &info.next_hop {
        p.insert("next_hop".into(), rmpv::Value::from(next_hop.as_str()));
    }
    let mut observation = Vec::new();
    append_observation(&mut observation, &info.observation, connection_generation);
    for (key, value) in observation {
        if let Some(key) = key.as_str() {
            p.insert(key.to_string(), value);
        }
    }
    ok_payload(p)
}

async fn dispatch_query_path_table(
    daemon: &Arc<dyn Daemon>,
    connection_generation: u64,
) -> Result<Payload, String> {
    let entries = daemon.query_path_table().await.map_err(|e| e.to_string())?;
    let paths: Vec<rmpv::Value> = entries
        .iter()
        .map(|info| {
            let mut m = Vec::new();
            m.push((
                rmpv::Value::from("destination_hash"),
                rmpv::Value::from(info.destination_hash.as_str()),
            ));
            if let Some(hops) = info.hops {
                m.push((rmpv::Value::from("hops"), rmpv::Value::from(hops as i64)));
            }
            if let Some(ref next_hop) = info.next_hop {
                m.push((rmpv::Value::from("next_hop"), rmpv::Value::from(next_hop.as_str())));
            }
            if let Some(ref iface) = info.interface {
                m.push((rmpv::Value::from("interface"), rmpv::Value::from(iface.as_str())));
            }
            if let Some(expires) = info.expires {
                m.push((rmpv::Value::from("expires"), rmpv::Value::from(expires)));
            }
            append_observation(&mut m, &info.observation, connection_generation);
            rmpv::Value::Map(m)
        })
        .collect();
    let mut p = Payload::new();
    p.insert("paths".into(), rmpv::Value::Array(paths));
    p.insert("count".into(), rmpv::Value::from(entries.len() as i64));
    ok_payload(p)
}

async fn dispatch_query_interface_stats(
    daemon: &Arc<dyn Daemon>,
    connection_generation: u64,
) -> Result<Payload, String> {
    let interfaces = daemon.list_interfaces().await.map_err(|e| e.to_string())?;
    let ifaces: Vec<rmpv::Value> = interfaces
        .iter()
        .map(|iface| {
            let mut m = vec![
                (rmpv::Value::from("name"), rmpv::Value::from(iface.name.as_str())),
                (rmpv::Value::from("hash"), rmpv::Value::from(iface.hash.as_str())),
                (rmpv::Value::from("type"), rmpv::Value::from(iface.kind.as_str())),
                (rmpv::Value::from("mode"), rmpv::Value::from(iface.mode.as_str())),
                (rmpv::Value::from("status"), rmpv::Value::from(iface.status.as_str())),
                (rmpv::Value::from("enabled"), rmpv::Value::from(iface.enabled)),
            ];
            if let Some(host) = &iface.host {
                m.push((rmpv::Value::from("host"), rmpv::Value::from(host.as_str())));
            }
            if let Some(port) = iface.port {
                m.push((rmpv::Value::from("port"), rmpv::Value::from(port as i64)));
            }
            if let Some(endpoint) = &iface.local_endpoint {
                m.push((rmpv::Value::from("local_endpoint"), rmpv::Value::from(endpoint.as_str())));
            }
            if let Some(endpoint) = &iface.remote_endpoint {
                m.push((
                    rmpv::Value::from("remote_endpoint"),
                    rmpv::Value::from(endpoint.as_str()),
                ));
            }
            if let Some(parent) = &iface.parent_hash {
                m.push((rmpv::Value::from("parent_hash"), rmpv::Value::from(parent.as_str())));
            }
            m.push((rmpv::Value::from("tx_bytes"), rmpv::Value::from(iface.tx_bytes)));
            m.push((rmpv::Value::from("rx_bytes"), rmpv::Value::from(iface.rx_bytes)));
            m.push((
                rmpv::Value::from("connected_peers"),
                rmpv::Value::from(iface.peers_connected as i64),
            ));
            append_observation(&mut m, &iface.observation, connection_generation);
            if let Some(failure) = &iface.failure {
                let code = match failure.code {
                    styrene_ipc::types::InterfaceFailureCode::Retrying => "retrying",
                    styrene_ipc::types::InterfaceFailureCode::Closed => "closed",
                    styrene_ipc::types::InterfaceFailureCode::UnknownState => "unknown_state",
                    _ => "unknown",
                };
                m.push((
                    rmpv::Value::from("failure"),
                    rmpv::Value::Map(vec![
                        (rmpv::Value::from("code"), rmpv::Value::from(code)),
                        (rmpv::Value::from("retryable"), rmpv::Value::from(failure.retryable)),
                    ]),
                ));
            }
            rmpv::Value::Map(m)
        })
        .collect();
    let mut p = Payload::new();
    p.insert("interfaces".into(), rmpv::Value::Array(ifaces));
    ok_payload(p)
}

// ── Remote Fleet Queries ─────────────────────────────────────────────────────

async fn dispatch_remote_inbox(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let dest = val_str(payload, "destination_hash").ok_or("missing destination_hash")?;
    let dest = validate_peer_hash(dest)?;
    let limit = val_u64(payload, "limit").unwrap_or(50) as u32;
    let timeout = val_u64(payload, "timeout");
    let conversations =
        daemon.remote_inbox(dest, limit, timeout).await.map_err(|e| e.to_string())?;
    let items = conversations.iter().map(conversation_info_value).collect();
    let mut p = Payload::new();
    p.insert("conversations".into(), rmpv::Value::Array(items));
    ok_payload(p)
}

async fn dispatch_remote_messages(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let dest = val_str(payload, "destination_hash").ok_or("missing destination_hash")?;
    let dest = validate_peer_hash(dest)?;
    let peer = val_str(payload, "peer_hash").ok_or("missing peer_hash")?;
    let peer = validate_peer_hash(peer)?;
    let limit = val_u64(payload, "limit").unwrap_or(50) as u32;
    let timeout = val_u64(payload, "timeout");
    let messages =
        daemon.remote_messages(dest, peer, limit, timeout).await.map_err(|e| e.to_string())?;
    let items: Vec<rmpv::Value> = messages
        .iter()
        .map(|m| {
            rmpv::Value::Map(vec![
                (rmpv::Value::from("id"), rmpv::Value::from(m.id.as_str())),
                (rmpv::Value::from("source_hash"), rmpv::Value::from(m.source_hash.as_str())),
                (rmpv::Value::from("content"), rmpv::Value::from(m.content.as_str())),
                (rmpv::Value::from("timestamp"), rmpv::Value::from(m.timestamp)),
            ])
        })
        .collect();
    let mut p = Payload::new();
    p.insert("messages".into(), rmpv::Value::Array(items));
    ok_payload(p)
}

// ── Self Update ──────────────────────────────────────────────────────────────

async fn dispatch_self_update(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let dest = val_str(payload, "destination_hash").ok_or("missing destination_hash")?;
    let dest = validate_peer_hash(dest)?;
    let version = val_str(payload, "version");
    let timeout = val_u64(payload, "timeout");
    let result = daemon.self_update(dest, version, timeout).await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("accepted".into(), rmpv::Value::Boolean(result.accepted));
    if let Some(v) = &result.current_version {
        p.insert("current_version".into(), rmpv::Value::from(v.as_str()));
    }
    if let Some(v) = &result.target_version {
        p.insert("target_version".into(), rmpv::Value::from(v.as_str()));
    }
    ok_payload(p)
}

// ── Tunnel ─────────────────────────────────────────────────────────────────

async fn dispatch_query_tunnels(daemon: &Arc<dyn Daemon>) -> Result<Payload, String> {
    let tunnels = daemon.list_tunnels().await.map_err(|e| e.to_string())?;
    let list: Vec<rmpv::Value> = tunnels
        .iter()
        .map(|t| {
            rmpv::Value::Map(vec![
                (rmpv::Value::from("peer_hash"), rmpv::Value::from(t.peer_hash.as_str())),
                (rmpv::Value::from("backend"), rmpv::Value::from(t.backend.as_str())),
                (rmpv::Value::from("state"), rmpv::Value::from(t.state.as_str())),
                (
                    rmpv::Value::from("remote_endpoint"),
                    rmpv::Value::from(t.remote_endpoint.as_deref().unwrap_or("")),
                ),
                (
                    rmpv::Value::from("established_at"),
                    rmpv::Value::from(t.established_at.unwrap_or(0)),
                ),
            ])
        })
        .collect();
    let mut p = Payload::new();
    p.insert("tunnels".into(), rmpv::Value::Array(list));
    ok_payload(p)
}

async fn dispatch_query_tunnel_status(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer = val_str(payload, "peer_hash").ok_or("missing peer_hash")?;
    let peer = validate_peer_hash(peer)?;
    let info = match daemon.tunnel_operation(peer).await {
        Ok(operation) => {
            let mut p = Payload::new();
            p.insert("operation_id".into(), rmpv::Value::from(operation.operation_id.as_str()));
            p.insert("peer_hash".into(), rmpv::Value::from(operation.peer_hash.as_str()));
            p.insert("kind".into(), rmpv::Value::from(operation.kind.as_str()));
            p.insert("state".into(), rmpv::Value::from(operation.state.as_str()));
            if let Some(code) = operation.error_code {
                p.insert("error_code".into(), rmpv::Value::from(code));
            }
            if let Some(message) = operation.error_message {
                p.insert("error_message".into(), rmpv::Value::from(message));
            }
            return ok_payload(p);
        }
        Err(_) => daemon.tunnel_status(peer).await.map_err(|e| e.to_string())?,
    };
    let mut p = Payload::new();
    p.insert("peer_hash".into(), rmpv::Value::from(info.peer_hash.as_str()));
    p.insert("backend".into(), rmpv::Value::from(info.backend.as_str()));
    p.insert("state".into(), rmpv::Value::from(info.state.as_str()));
    p.insert(
        "remote_endpoint".into(),
        rmpv::Value::from(info.remote_endpoint.as_deref().unwrap_or("")),
    );
    p.insert("established_at".into(), rmpv::Value::from(info.established_at.unwrap_or(0)));
    ok_payload(p)
}

async fn dispatch_tunnel_teardown(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer = val_str(payload, "peer_hash").ok_or("missing peer_hash")?;
    let peer = validate_peer_hash(peer)?;
    let ok = daemon.tunnel_teardown(peer).await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("success".into(), rmpv::Value::Boolean(ok));
    ok_payload(p)
}

// ── Fleet Apply ─────────────────────────────────────────────────────────────

async fn dispatch_fleet_apply(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let dest = val_str(payload, "destination_hash").ok_or("missing destination_hash")?;
    let dest = validate_peer_hash(dest)?;
    let profile_b64 = val_str(payload, "profile").ok_or("missing profile")?;
    let profile_bytes = base64::engine::general_purpose::STANDARD
        .decode(profile_b64)
        .map_err(|e| format!("invalid base64 profile: {e}"))?;
    // Issue 4: Reject oversized profiles after base64 decode
    if profile_bytes.len() > 4 * 1024 * 1024 {
        return Err("decoded profile exceeds 4 MB limit".into());
    }
    let verify = payload.get("verify").and_then(|v| v.as_bool()).unwrap_or(true);
    let timeout = payload.get("timeout").and_then(|v| v.as_u64());

    let result = daemon
        .fleet_apply(dest, profile_bytes, verify, timeout)
        .await
        .map_err(|e| e.to_string())?;

    let mut p = Payload::new();
    p.insert("success".into(), rmpv::Value::Boolean(result.success));
    p.insert("verified".into(), rmpv::Value::Boolean(result.verified));
    p.insert("exit_code".into(), rmpv::Value::from(result.exit_code));
    p.insert("stdout".into(), rmpv::Value::String(result.stdout.into()));
    p.insert("stderr".into(), rmpv::Value::String(result.stderr.into()));
    ok_payload(p)
}

// ── Tunnel Establish ───────────────────────────────────────────────────────

async fn dispatch_tunnel_establish(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let peer = val_str(payload, "peer_hash").ok_or("missing peer_hash")?;
    let peer = validate_peer_hash(peer)?;
    let nonce = daemon.tunnel_establish(peer).await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("accepted".into(), rmpv::Value::Boolean(true));
    p.insert("success".into(), rmpv::Value::Boolean(true));
    p.insert("operation_id".into(), rmpv::Value::from(nonce.as_str()));
    p.insert("peer_hash".into(), rmpv::Value::from(peer));
    p.insert("state".into(), rmpv::Value::from("queued"));
    p.insert("nonce".into(), rmpv::Value::from(nonce.as_str()));
    ok_payload(p)
}

// ── Fleet Grant / Revoke ───────────────────────────────────────────────────

async fn dispatch_fleet_grant(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let identity_hash = val_str(payload, "identity_hash").ok_or("missing identity_hash")?;
    let identity_hash = validate_peer_hash(identity_hash)?;
    let role = val_str(payload, "role").ok_or("missing role")?.to_string();
    let label = val_str(payload, "label").unwrap_or("").to_string();
    let grants: Vec<String> = payload
        .get("grants")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let ok = daemon
        .fleet_grant(identity_hash, &role, &label, grants)
        .await
        .map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("success".into(), rmpv::Value::Boolean(ok));
    ok_payload(p)
}

async fn dispatch_fleet_revoke(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let identity_hash = val_str(payload, "identity_hash").ok_or("missing identity_hash")?;
    let identity_hash = validate_peer_hash(identity_hash)?;
    let ok = daemon.fleet_revoke(identity_hash).await.map_err(|e| e.to_string())?;
    let mut p = Payload::new();
    p.insert("success".into(), rmpv::Value::Boolean(ok));
    ok_payload(p)
}

// ── Page Operations ────────────────────────────────────────────────────

async fn dispatch_query_page(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    owner: u64,
) -> Result<Payload, String> {
    let host = val_str(payload, "host").unwrap_or("local");
    let path = val_str(payload, "path").unwrap_or("/");
    let timeout = payload.get("timeout").and_then(|v| v.as_u64());

    let page = daemon
        .browse_page_for_owner(owner, host, path, timeout)
        .await
        .map_err(|e| e.to_string())?;
    page_payload(page, owner)
}

fn decode_typed<T: serde::de::DeserializeOwned>(payload: &Payload, key: &str) -> Result<T, String> {
    let bytes = payload
        .get(key)
        .and_then(rmpv::Value::as_slice)
        .ok_or_else(|| format!("missing typed {key} payload"))?;
    rmp_serde::from_slice(bytes).map_err(|error| format!("decode {key}: {error}"))
}

fn typed_payload<T: serde::Serialize>(key: &str, value: &T) -> Result<Payload, String> {
    let bytes = rmp_serde::to_vec_named(value).map_err(|error| format!("encode {key}: {error}"))?;
    let mut payload = Payload::new();
    payload.insert(key.into(), rmpv::Value::Binary(bytes));
    ok_payload(payload)
}

async fn dispatch_page_navigate(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    owner: u64,
) -> Result<Payload, String> {
    let request = decode_typed(payload, "navigation")?;
    let page =
        daemon.navigate_page_for_owner(owner, request).await.map_err(|error| error.to_string())?;
    page_payload(page, owner)
}

async fn dispatch_page_disconnect(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    owner: u64,
) -> Result<Payload, String> {
    let session_id = val_str(payload, "session_id").ok_or("missing session_id")?;
    let navigation = daemon
        .close_page_session_for_owner(owner, session_id)
        .await
        .map_err(|error| error.to_string())?;
    typed_payload("navigation", &navigation)
}

async fn dispatch_file_download_start(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    owner: u64,
) -> Result<Payload, String> {
    let request = decode_typed(payload, "download_request")?;
    let download = daemon
        .start_file_download_for_owner(owner, request)
        .await
        .map_err(|error| error.to_string())?;
    typed_payload("download", &download)
}

async fn dispatch_file_download_query(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    owner: u64,
) -> Result<Payload, String> {
    let id = val_str(payload, "download_id").ok_or("missing download_id")?;
    let download =
        daemon.file_download_for_owner(owner, id).await.map_err(|error| error.to_string())?;
    typed_payload("download", &download)
}

async fn dispatch_file_download_cancel(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    owner: u64,
) -> Result<Payload, String> {
    let id = val_str(payload, "download_id").ok_or("missing download_id")?;
    let download = daemon
        .cancel_file_download_for_owner(owner, id)
        .await
        .map_err(|error| error.to_string())?;
    typed_payload("download", &download)
}

async fn dispatch_file_download_save(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
    owner: u64,
) -> Result<Payload, String> {
    let id = val_str(payload, "download_id").ok_or("missing download_id")?;
    let destination = val_str(payload, "destination").ok_or("missing destination")?;
    let download = daemon
        .save_file_download_for_owner(owner, id, destination)
        .await
        .map_err(|error| error.to_string())?;
    typed_payload("download", &download)
}

fn page_payload(
    mut page: styrene_ipc::types::PageContent,
    connection_generation: u64,
) -> Result<Payload, String> {
    if connection_generation != 0 {
        page.observation.connection_generation = Some(connection_generation);
        for stage in &mut page.stages {
            stage.observation.connection_generation = Some(connection_generation);
        }
    }
    let encoded_page = rmp_serde::to_vec_named(&page)
        .map_err(|error| format!("page IPC projection encoding failed: {error}"))?;
    let mut p = Payload::new();
    p.insert("page".into(), rmpv::Value::Binary(encoded_page));
    let encoded_payload = rmp_serde::to_vec(&p)
        .map_err(|error| format!("page IPC payload encoding failed: {error}"))?;
    if encoded_payload.len() > crate::wire::MAX_PAYLOAD_SIZE {
        return Err(format!(
            "page IPC payload exceeds {} byte limit",
            crate::wire::MAX_PAYLOAD_SIZE
        ));
    }
    ok_payload(p)
}

async fn dispatch_list_pages(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let host = val_str(payload, "host").unwrap_or("local");
    let timeout = payload.get("timeout").and_then(|v| v.as_u64());

    let pages = daemon.list_pages(host, timeout).await.map_err(|e| e.to_string())?;

    page_list_payload(&pages)
}

fn page_list_payload(pages: &[styrene_ipc::types::PageInfo]) -> Result<Payload, String> {
    let mut response = Payload::new();
    response.insert("pages".into(), serialized_value(&pages)?);
    ok_payload(response)
}

// ── Terminal Operations ────────────────────────────────────────────────

async fn dispatch_terminal_open(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let _dest = val_str(payload, "destination_hash").unwrap_or("local");
    let _rows = payload.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u16;
    let _cols = payload.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u16;

    let mut request = styrene_ipc::types::TerminalOpenRequest::default();
    request.destination = _dest.to_string();
    request.rows = _rows;
    request.cols = _cols;

    let session_id = daemon.terminal_open(request).await.map_err(|e| e.to_string())?;

    let mut p = Payload::new();
    p.insert("session_id".into(), rmpv::Value::from(session_id.as_str()));
    ok_payload(p)
}

async fn dispatch_terminal_input(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let session_id = val_str(payload, "session_id").ok_or("missing session_id")?;
    let data = payload.get("data").and_then(|v| v.as_slice()).unwrap_or(&[]);

    daemon.terminal_input(session_id, data).await.map_err(|e| e.to_string())?;

    ok_payload(Payload::new())
}

async fn dispatch_terminal_resize(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let session_id = val_str(payload, "session_id").ok_or("missing session_id")?;
    let rows = payload.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u16;
    let cols = payload.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u16;

    daemon.terminal_resize(session_id, rows, cols).await.map_err(|e| e.to_string())?;

    ok_payload(Payload::new())
}

async fn dispatch_terminal_close(
    daemon: &Arc<dyn Daemon>,
    payload: &Payload,
) -> Result<Payload, String> {
    let session_id = val_str(payload, "session_id").ok_or("missing session_id")?;

    daemon.terminal_close(session_id).await.map_err(|e| e.to_string())?;

    ok_payload(Payload::new())
}

#[cfg(test)]
mod page_projection_tests {
    use super::*;
    use styrene_ipc::types::{
        PageBrowseStage, PageBrowseStageKind, PageBrowseStageState, PageCacheStatus,
        PageParserWarning, PageTransferKind,
    };

    #[test]
    fn typed_page_projection_retains_all_authoritative_metadata() {
        let mut page = styrene_ipc::types::PageContent::default();
        page.source_bytes = b">Index\nbody".to_vec();
        page.rendered_text = "Index\nbody".into();
        page.title = Some("Index".into());
        page.links = vec!["next.mu".into()];
        page.correlation_id = "page-correlation".into();
        page.source_checksum = "ab".repeat(32);
        page.request.native_path = "/page/index.mu".into();
        page.request.path_hash = "cd".repeat(16);
        page.request.request_id = Some("ef".repeat(16));
        page.transfer.kind = PageTransferKind::Resource;
        page.transfer.resource_hash = Some("12".repeat(32));
        page.transfer.verified = true;
        page.cache.status = PageCacheStatus::Hit;
        page.cache.stored_at = Some(42);
        let mut stage = PageBrowseStage::default();
        stage.correlation_id = page.correlation_id.clone();
        stage.kind = PageBrowseStageKind::Transfer;
        stage.state = PageBrowseStageState::Succeeded;
        page.stages.push(stage);
        let mut warning = PageParserWarning::default();
        warning.code = "unsupported".into();
        warning.message = "retained".into();
        page.parser_warnings.push(warning);

        let mut expected = page.clone();
        expected.observation.connection_generation = Some(41);
        expected.stages[0].observation.connection_generation = Some(41);
        let payload = page_payload(page, 41).expect("IPC-safe projection");
        let bytes = match payload.get("page") {
            Some(rmpv::Value::Binary(bytes)) => bytes,
            _ => panic!("typed page binary missing"),
        };
        let decoded: styrene_ipc::types::PageContent =
            rmp_serde::from_slice(bytes).expect("typed page decode");

        assert_eq!(decoded, expected);
        assert_eq!(decoded.observation.connection_generation, Some(41));
        assert_eq!(decoded.stages[0].observation.connection_generation, Some(41));
    }

    #[test]
    fn oversized_typed_page_returns_explicit_error() {
        let mut page = styrene_ipc::types::PageContent::default();
        page.source_bytes = vec![0; crate::wire::MAX_PAYLOAD_SIZE];

        let error = page_payload(page, 0).expect_err("oversized IPC projection must fail");

        assert!(error.contains("exceeds"));
    }

    #[test]
    fn local_inventory_projection_retains_native_handler_metadata() {
        let mut page = styrene_ipc::types::PageInfo::default();
        page.path = "/file/manual.bin".into();
        page.title = Some("Manual".into());
        page.host_hash = "11".repeat(16);
        page.kind = "file".into();
        page.dynamic = false;
        page.restricted = true;
        page.handler_active = true;

        let payload = page_list_payload(&[page.clone()]).expect("inventory projection");
        let decoded: Vec<styrene_ipc::types::PageInfo> = payload
            .get("pages")
            .and_then(rmpv::Value::as_array)
            .expect("pages array")
            .iter()
            .cloned()
            .map(|value| rmpv::ext::from_value(value).expect("page entry"))
            .collect();

        assert_eq!(decoded, [page]);
    }
}

#[cfg(test)]
mod conversation_projection_tests {
    use super::*;

    #[test]
    fn unread_only_and_legacy_alias_have_consistent_filter_semantics() {
        assert!(!conversation_unread_only(&Payload::new()).unwrap());
        assert!(
            conversation_unread_only(&Payload::from([(
                "unread_only".into(),
                rmpv::Value::Boolean(true),
            )]))
            .unwrap()
        );
        assert!(
            !conversation_unread_only(&Payload::from([(
                "unread_only".into(),
                rmpv::Value::Boolean(false),
            )]))
            .unwrap()
        );
        assert!(
            conversation_unread_only(&Payload::from([(
                "include_unread".into(),
                rmpv::Value::Boolean(true),
            )]))
            .unwrap()
        );
    }

    #[test]
    fn shared_conversation_projection_retains_additive_fields() {
        let mut conversation = styrene_ipc::types::ConversationInfo::default();
        conversation.peer_hash = "ab".repeat(16);
        conversation.peer_name = Some("Peer".into());
        conversation.last_message_timestamp = Some(42);
        conversation.last_message_content = Some("hello".into());
        conversation.unread_count = 3;
        conversation.message_count = 7;
        conversation.pinned = true;
        conversation.muted = true;

        let rmpv::Value::Map(fields) = conversation_info_value(&conversation) else {
            panic!("conversation projection must be a map");
        };
        let field = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key.as_str() == Some(name))
                .map(|(_, value)| value)
                .expect("projected field")
        };
        assert_eq!(field("peer_name").as_str(), Some("Peer"));
        assert_eq!(field("last_message_timestamp").as_i64(), Some(42));
        assert_eq!(field("last_message_content").as_str(), Some("hello"));
        assert_eq!(field("unread_count").as_u64(), Some(3));
        assert_eq!(field("message_count").as_u64(), Some(7));
        assert_eq!(field("pinned").as_bool(), Some(true));
        assert_eq!(field("muted").as_bool(), Some(true));
    }
}

#[cfg(test)]
mod propagation_correlation_projection_tests {
    use super::*;

    #[test]
    fn message_correlation_is_bounded_metadata_without_payload_or_stamp() {
        let mut message = styrene_ipc::types::MessageInfo::default();
        let mut correlation = styrene_ipc::types::MessagePropagationCorrelationInfo::default();
        correlation.relation = "inbound".into();
        correlation.transient_id = "11".repeat(32);
        correlation.attempt_id = Some("22".repeat(16));
        correlation.peer_hash = Some("33".repeat(16));
        correlation.state = "pending_ack".into();
        correlation.created_at = 1;
        correlation.updated_at = 2;
        message.propagation_correlations.push(correlation);

        let rmpv::Value::Map(fields) = message_info_value(&message) else {
            panic!("message projection must be a map");
        };
        let correlations = fields
            .iter()
            .find_map(|(key, value)| {
                (key.as_str() == Some("propagation_correlations")).then_some(value)
            })
            .expect("correlation field");
        let encoded = format!("{correlations:?}");
        assert!(encoded.contains("transient_id"));
        assert!(encoded.contains("attempt_id"));
        assert!(!encoded.contains("payload"));
        assert!(!encoded.contains("stamp"));
    }
}
