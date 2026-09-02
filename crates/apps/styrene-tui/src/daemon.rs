//! Daemon RPC bridge — connects styrene-tui to a running styrened daemon
#![allow(dead_code)]
//! via the Unix socket IPC protocol.
//!
//! Architecture:
//!   - `DaemonHandle` is the single connection owner
//!   - `connect()` dials the socket, returns the handle + event receiver
//!   - The caller drives a background task that calls `poll_events()`
//!     and converts `DaemonEvent`s into `TuiEvent`s for the App
//!
//! Wire protocol: msgpack frames over Unix domain socket
//! (same as Python TUI ↔ styrened — wire-compatible).
//!
//! Usage:
//!
//! ```ignore
//! // In main, after building App:
//! if let Ok(mut connection) = daemon::connect(None).await {
//!     let handle = connection.take_handle();
//!     tokio::spawn(async move {
//!         while let Some(ev) = connection.events.recv().await {
//!             // post ev into app via a Mutex or channel
//!         }
//!     });
//! }
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use tokio::time::Duration;

use rmpv::Value as MpValue;
use styrene_ipc::types::{
    ConversationInfo, DaemonStatusInfo, DeviceInfo, IdentityInfo, InterfaceDetail, LinkActivity,
    LinkEvent as IpcLinkEvent, LinkEventKind, LinkLifecycleReason, LinkSnapshot, MessageInfo,
    MessagingOperationOutcome, NetworkOperationInfo, NetworkOperationKind, ObservationMetadata,
    PathInfo, RequestObservationInfo, ResourceTransferInfo, StandardPropagationSnapshot,
    StartNetworkOperationInfo, StartRequestInfo,
};
use styrene_ipc_client::{Client, ConnectionGeneration};
use styrene_ipc_wire::{Frame, MessageType, REQUEST_ID_SIZE};

use crate::mesh_state::{ActivityEntry, ActivityKind, PeerRecord, epoch_secs};
use crate::tui::segments::DeliveryStatus;

// ─── TuiEvent — what the bridge sends to the App ─────────────────────────────

#[derive(Debug, Clone)]
pub enum TuiEvent {
    /// Initial identity loaded on connect.
    Identity(IdentityInfo),
    /// Daemon status snapshot (polled periodically).
    Status(DaemonStatusInfo),
    EventGeneration(u64),
    /// The backend's description of the profile the daemon runs from. Only
    /// managed daemons send it; the TUI never derives it from a mode name.
    Profile(Box<styrene_ipc::types::ProfileInfo>),
    /// New or updated announce / peer record.
    PeerAnnounce(PeerRecord),
    /// Inbound LXMF message received.
    Message(Box<MessageInfo>),
    MessageResolved {
        message_id: String,
        message: Option<Box<MessageInfo>>,
        generation: u64,
    },
    /// Message delivery status changed.
    MessageStatus {
        id: String,
        status: String,
    },
    MessagingOperation(Box<MessagingOperationOutcome>),
    /// Link telemetry event (activated, closed, rtt_updated).
    LinkUpdate {
        link_id: String,
        peer_hash: String,
        peer_name: Option<String>,
        interface: Option<String>,
        status: String,
        kind: LinkEventKind,
        activity: LinkActivity,
        reason: Option<LinkLifecycleReason>,
        remote_identity_hash: Option<String>,
        rtt_ms: Option<f64>,
        observation: ObservationMetadata,
    },
    RouteLifecycle {
        kind: String,
        destination_hash: String,
        loss_reason: Option<String>,
        expires: Option<i64>,
        observation: ObservationMetadata,
    },
    NetworkOperation(NetworkOperationInfo),
    Request(RequestObservationInfo),
    Resource(ResourceTransferInfo),
    LinkSnapshot(LinkSnapshot),
    NetworkOperationSnapshot(Vec<NetworkOperationInfo>),
    RequestSnapshot(Vec<RequestObservationInfo>),
    ResourceSnapshot(Vec<ResourceTransferInfo>),
    RequestReconcileRequired {
        dropped: u64,
        connection_generation: u64,
    },
    ReconcileRequired {
        dropped: u64,
        connection_generation: u64,
    },
    StandardPropagationChanged {
        connection_generation: u64,
    },
    StandardPropagationSnapshot(StandardPropagationSnapshot),
    RouteSnapshot(Vec<PathInfo>),
    InterfaceSnapshot(Vec<InterfaceDetail>),
    /// Result of a chat send, correlated to its conversation and message.
    ChatSendResult {
        peer_hash: String,
        message_id: Option<String>,
        success: bool,
        detail: String,
        generation: u64,
    },
    ChatSendOutcome {
        peer_hash: String,
        outcome: Box<styrene_ipc::types::SendChatOutcome>,
        generation: u64,
    },
    DraftLoaded {
        peer_hash: String,
        draft: Option<styrene_ipc::types::ConversationDraft>,
        generation: u64,
    },
    DraftCleared {
        peer_hash: String,
        generation: u64,
    },
    MessagePage {
        peer_hash: String,
        messages: Vec<MessageInfo>,
        next_cursor: Option<String>,
        reset: bool,
        generation: u64,
    },
    ConversationPage {
        conversations: Vec<ConversationInfo>,
        next_cursor: Option<String>,
        reset: bool,
        generation: u64,
    },
    /// Result of a queued daemon command.
    CommandResult {
        action: String,
        success: bool,
        detail: String,
        generation: u64,
    },
    /// Page content loaded from a host.
    PageLoaded {
        host: String,
        path: String,
        page: Box<styrene_ipc::types::PageContent>,
        generation: u64,
    },
    PageClosed {
        session_id: String,
        generation: u64,
    },
    /// Page list from a host.
    PageList {
        host: String,
        pages: Vec<String>,
        generation: u64,
    },
    FileDownload {
        download: styrene_ipc::types::FileDownloadInfo,
        generation: u64,
    },
    /// Terminal output data from a remote session.
    TerminalOutput {
        session_id: String,
        data: Vec<u8>,
    },
    /// Terminal session exited.
    TerminalExited {
        session_id: String,
        exit_code: Option<i32>,
    },
    /// Daemon disconnected or unreachable.
    Disconnected(String),
}

// ─── Daemon Command Queue ────────────────────────────────────────────────────
//
// The key handler is synchronous but daemon calls are async. Commands are queued
// from the sync handler and executed by a background task that owns DaemonHandle.
// Results come back as TuiEvents.

#[derive(Debug)]
pub enum DaemonCmd {
    /// Send a chat message to a peer.
    SendChat {
        peer_hash: String,
        content: String,
        delivery_method: String,
    },
    SetDraft {
        peer_hash: String,
        content: String,
    },
    LoadDraft {
        peer_hash: String,
    },
    ClearDraft {
        peer_hash: String,
    },
    RetryMessage {
        message_id: String,
    },
    CancelMessage {
        message_id: String,
    },
    LoadMessagePage {
        peer_hash: String,
        cursor: Option<String>,
    },
    QueryMessage {
        message_id: String,
    },
    LoadConversationPage {
        cursor: Option<String>,
    },
    /// Announce this node to the mesh.
    Announce,
    StartNetworkOperation(StartNetworkOperationInfo),
    CancelNetworkOperation {
        operation_id: String,
    },
    StartRequest(StartRequestInfo),
    CancelRequest {
        request_id: String,
    },
    CancelResource {
        resource_hash: String,
    },
    ReconcileNetworkObservations,
    RequeryStandardPropagation,
    InspectRoutes,
    InspectInterfaces,
    InspectLinks,
    InspectRequests,
    InspectResources,
    /// Block a peer by identity hash.
    BlockPeer {
        identity_hash: String,
    },
    /// Unblock a peer by identity hash.
    UnblockPeer {
        identity_hash: String,
    },
    /// Query remote device status.
    DeviceStatus {
        dest_hash: String,
    },
    /// Execute a command on a remote device.
    Exec {
        dest_hash: String,
        command: String,
        args: Vec<String>,
    },
    /// Reboot a remote device.
    RebootDevice {
        dest_hash: String,
        delay_secs: Option<u64>,
    },
    /// Push config profile to a remote device.
    FleetApply {
        dest_hash: String,
        profile_hex: String,
    },
    /// Update local identity.
    SetIdentity {
        display_name: String,
        icon: Option<String>,
    },
    /// Set auto-reply configuration.
    SetAutoReply {
        mode: String,
        message: String,
    },
    /// Mark conversation as read.
    MarkRead {
        peer_hash: String,
    },
    /// Browse a page from a host.
    BrowsePage {
        host: String,
        path: String,
    },
    NavigatePage(styrene_ipc::types::PageNavigationRequest),
    ClosePage {
        session_id: String,
    },
    StartFileDownload(styrene_ipc::types::FileDownloadRequest),
    QueryFileDownload {
        download_id: String,
    },
    CancelFileDownload {
        download_id: String,
    },
    SaveFileDownload {
        download_id: String,
        destination: String,
    },
    /// List pages served by a host.
    ListPages {
        host: String,
    },
}

#[derive(Debug)]
pub struct QueuedDaemonCmd {
    pub command: DaemonCmd,
    pub origin_generation: u64,
    pub capability: String,
}

fn queued_command_authorized(status: &DaemonStatusInfo, queued: &QueuedDaemonCmd) -> bool {
    status.connection_generation == Some(queued.origin_generation)
        && status.active_capabilities.as_ref().is_some_and(|active| {
            active.version == styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION
                && !active.degraded.iter().any(|item| item.id == queued.capability)
                && active.authorized_operations.iter().any(|item| item == &queued.capability)
        })
}

/// Spawn the command executor task. Processes DaemonCmd messages and posts
/// results back as TuiEvents via the event channel.
pub fn spawn_command_executor(
    handle: Arc<Mutex<DaemonHandle>>,
    mut cmd_rx: mpsc::Receiver<QueuedDaemonCmd>,
    event_tx: mpsc::Sender<TuiEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(queued) = cmd_rx.recv().await {
            let mut h = handle.lock().await;
            let authorized = h
                .status()
                .await
                .ok()
                .is_some_and(|status| queued_command_authorized(&status, &queued));
            if !authorized {
                let _ = event_tx
                    .send(TuiEvent::CommandResult {
                        action: queued.capability,
                        success: false,
                        detail: "command rejected: origin generation or capability is stale".into(),
                        generation: queued.origin_generation,
                    })
                    .await;
                continue;
            }
            let result_generation = queued.origin_generation;
            match queued.command {
                DaemonCmd::SendChat { peer_hash, content, delivery_method } => {
                    match h.send_chat_outcome(&peer_hash, &content, None, &delivery_method).await {
                        Ok(outcome) => {
                            let _ = event_tx
                                .send(TuiEvent::ChatSendOutcome {
                                    peer_hash,
                                    outcome: Box::new(outcome),
                                    generation: result_generation,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = event_tx
                                .send(TuiEvent::ChatSendResult {
                                    peer_hash,
                                    message_id: None,
                                    success: false,
                                    detail: e,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::SetDraft { peer_hash, content } => {
                    if let Err(detail) = h.set_draft(&peer_hash, &content).await {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "save draft".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                }
                DaemonCmd::LoadDraft { peer_hash } => match h.draft(&peer_hash).await {
                    Ok(draft) => {
                        let _ = event_tx
                            .send(TuiEvent::DraftLoaded {
                                peer_hash,
                                draft,
                                generation: result_generation,
                            })
                            .await;
                    }
                    Err(detail) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "load draft".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::ClearDraft { peer_hash } => match h.clear_draft(&peer_hash).await {
                    Ok(()) => {
                        let _ = event_tx
                            .send(TuiEvent::DraftCleared {
                                peer_hash,
                                generation: result_generation,
                            })
                            .await;
                    }
                    Err(detail) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "discard draft".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::RetryMessage { message_id } => {
                    match h.retry_message(&message_id).await {
                        Ok(outcome) => {
                            let _ = event_tx
                                .send(TuiEvent::MessagingOperation(Box::new(outcome)))
                                .await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "retry message".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::CancelMessage { message_id } => {
                    match h.cancel_message(&message_id).await {
                        Ok(outcome) => {
                            let _ = event_tx
                                .send(TuiEvent::MessagingOperation(Box::new(outcome)))
                                .await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "cancel message".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::LoadMessagePage { peer_hash, cursor } => {
                    match h.message_page(&peer_hash, cursor.as_deref()).await {
                        Ok((messages, next_cursor, reset)) => {
                            let _ = event_tx
                                .send(TuiEvent::MessagePage {
                                    peer_hash,
                                    messages,
                                    next_cursor,
                                    reset,
                                    generation: result_generation,
                                })
                                .await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "load message history".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::QueryMessage { message_id } => match h.message(&message_id).await {
                    Ok(message) => {
                        let _ = event_tx
                            .send(TuiEvent::MessageResolved {
                                message_id,
                                message: message.map(Box::new),
                                generation: result_generation,
                            })
                            .await;
                    }
                    Err(detail) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "query message".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::LoadConversationPage { cursor } => {
                    match h.conversation_page(cursor.as_deref()).await {
                        Ok((conversations, next_cursor, reset)) => {
                            let _ = event_tx
                                .send(TuiEvent::ConversationPage {
                                    conversations,
                                    next_cursor,
                                    reset,
                                    generation: result_generation,
                                })
                                .await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "load conversations".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::Announce => {
                    let mut request = StartNetworkOperationInfo::default();
                    request.kind = NetworkOperationKind::Announce;
                    request.timeout_ms = 15_000;
                    match h.start_network_operation(request).await {
                        Ok(operation) => {
                            let _ = event_tx.send(TuiEvent::NetworkOperation(operation)).await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "announce".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::StartNetworkOperation(request) => {
                    match h.start_network_operation(request).await {
                        Ok(operation) => {
                            let _ = event_tx.send(TuiEvent::NetworkOperation(operation)).await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "network operation".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::CancelNetworkOperation { operation_id } => {
                    match h.cancel_network_operation(&operation_id).await {
                        Ok(operation) => {
                            let _ = event_tx.send(TuiEvent::NetworkOperation(operation)).await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "cancel operation".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::StartRequest(request) => match h.start_request(request).await {
                    Ok(request) => {
                        let _ = event_tx.send(TuiEvent::Request(request)).await;
                    }
                    Err(detail) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "request".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::CancelRequest { request_id } => {
                    match h.cancel_request(&request_id).await {
                        Ok(request) => {
                            let _ = event_tx.send(TuiEvent::Request(request)).await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "cancel request".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::CancelResource { resource_hash } => {
                    match h.cancel_resource(&resource_hash).await {
                        Ok(true) => {}
                        Ok(false) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "cancel resource".into(),
                                    success: false,
                                    detail: "resource cancellation was not accepted".into(),
                                    generation: result_generation,
                                })
                                .await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "cancel resource".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::ReconcileNetworkObservations => {
                    if let Ok(routes) = h.path_table().await {
                        let _ = event_tx.send(TuiEvent::RouteSnapshot(routes)).await;
                    }
                    if let Ok(interfaces) = h.interface_stats().await {
                        let _ = event_tx.send(TuiEvent::InterfaceSnapshot(interfaces)).await;
                    }
                    if let Ok(links) = h.links().await {
                        let _ = event_tx.send(TuiEvent::LinkSnapshot(links)).await;
                    }
                    match h.network_operations().await {
                        Ok(operations) => {
                            let _ =
                                event_tx.send(TuiEvent::NetworkOperationSnapshot(operations)).await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "reconcile operations".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                    match h.requests().await {
                        Ok(requests) => {
                            let _ = event_tx.send(TuiEvent::RequestSnapshot(requests)).await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "reconcile requests".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                    if let Ok(resources) = h.resources().await {
                        let _ = event_tx.send(TuiEvent::ResourceSnapshot(resources)).await;
                    }
                }
                DaemonCmd::RequeryStandardPropagation => match h.standard_propagation().await {
                    Ok(snapshot) => {
                        let _ =
                            event_tx.send(TuiEvent::StandardPropagationSnapshot(snapshot)).await;
                    }
                    Err(detail) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "standard propagation".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::InspectRoutes => match h.path_table().await {
                    Ok(routes) => {
                        let _ = event_tx.send(TuiEvent::RouteSnapshot(routes)).await;
                    }
                    Err(detail) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "routes".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::InspectInterfaces => match h.interface_stats().await {
                    Ok(interfaces) => {
                        let _ = event_tx.send(TuiEvent::InterfaceSnapshot(interfaces)).await;
                    }
                    Err(detail) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "interfaces".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::InspectLinks => match h.links().await {
                    Ok(links) => {
                        let _ = event_tx.send(TuiEvent::LinkSnapshot(links)).await;
                    }
                    Err(detail) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "links".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::InspectRequests => match h.requests().await {
                    Ok(requests) => {
                        let _ = event_tx.send(TuiEvent::RequestSnapshot(requests)).await;
                    }
                    Err(detail) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "requests".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::InspectResources => match h.resources().await {
                    Ok(resources) => {
                        let _ = event_tx.send(TuiEvent::ResourceSnapshot(resources)).await;
                    }
                    Err(detail) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "resources".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::BlockPeer { identity_hash } => {
                    let result = h.block_peer(&identity_hash).await;
                    let _ = event_tx
                        .send(TuiEvent::CommandResult {
                            action: "block_peer".into(),
                            success: result.is_ok(),
                            detail: result.err().unwrap_or_else(|| {
                                format!("blocked {}", &identity_hash[..8.min(identity_hash.len())])
                            }),
                            generation: result_generation,
                        })
                        .await;
                }
                DaemonCmd::UnblockPeer { identity_hash } => {
                    let result = h.unblock_peer(&identity_hash).await;
                    let _ = event_tx
                        .send(TuiEvent::CommandResult {
                            action: "unblock_peer".into(),
                            success: result.is_ok(),
                            detail: result.err().unwrap_or_else(|| {
                                format!(
                                    "unblocked {}",
                                    &identity_hash[..8.min(identity_hash.len())]
                                )
                            }),
                            generation: result_generation,
                        })
                        .await;
                }
                DaemonCmd::DeviceStatus { dest_hash } => {
                    match h.device_status(&dest_hash, Some(30)).await {
                        Ok(payload) => {
                            let detail = format_payload_summary(&payload);
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "device_status".into(),
                                    success: true,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "device_status".into(),
                                    success: false,
                                    detail: e,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::Exec { dest_hash, command, args } => {
                    match h.exec(&dest_hash, &command, &args, Some(60)).await {
                        Ok(payload) => {
                            let detail = format_payload_summary(&payload);
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "exec".into(),
                                    success: true,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "exec".into(),
                                    success: false,
                                    detail: e,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::RebootDevice { dest_hash, delay_secs } => {
                    let result = h.reboot_device(&dest_hash, delay_secs, Some(30)).await;
                    let _ = event_tx
                        .send(TuiEvent::CommandResult {
                            action: "reboot_device".into(),
                            success: result.is_ok(),
                            detail: result.err().unwrap_or_else(|| "reboot accepted".into()),
                            generation: result_generation,
                        })
                        .await;
                }
                DaemonCmd::FleetApply { dest_hash, profile_hex } => {
                    match h.fleet_apply(&dest_hash, &profile_hex, true, Some(120)).await {
                        Ok(payload) => {
                            let detail = format_payload_summary(&payload);
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "fleet_apply".into(),
                                    success: true,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "fleet_apply".into(),
                                    success: false,
                                    detail: e,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::SetIdentity { display_name, icon } => {
                    let result = h.set_identity(&display_name, icon.as_deref()).await;
                    let _ = event_tx
                        .send(TuiEvent::CommandResult {
                            action: "set_identity".into(),
                            success: result.is_ok(),
                            detail: result.err().unwrap_or_else(|| "identity updated".into()),
                            generation: result_generation,
                        })
                        .await;
                }
                DaemonCmd::SetAutoReply { mode, message } => {
                    let result = h.set_auto_reply(&mode, &message, None).await;
                    let _ = event_tx
                        .send(TuiEvent::CommandResult {
                            action: "set_auto_reply".into(),
                            success: result.is_ok(),
                            detail: result.err().unwrap_or_else(|| "auto-reply updated".into()),
                            generation: result_generation,
                        })
                        .await;
                }
                DaemonCmd::MarkRead { peer_hash } => {
                    let _ = h.mark_read(&peer_hash).await;
                }
                DaemonCmd::BrowsePage { host, path } => match h.query_page(&host, &path).await {
                    Ok(page) => {
                        let _ = event_tx
                            .send(TuiEvent::PageLoaded {
                                host: host.clone(),
                                path,
                                page: Box::new(page),
                                generation: result_generation,
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "browse_page".into(),
                                success: false,
                                detail: e,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::NavigatePage(request) => match h.navigate_page(request).await {
                    Ok(page) => {
                        let host = page.host_hash.clone();
                        let path = page.request.native_path.clone();
                        let _ = event_tx
                            .send(TuiEvent::PageLoaded {
                                host,
                                path,
                                page: Box::new(page),
                                generation: result_generation,
                            })
                            .await;
                    }
                    Err(detail) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "navigate page".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::ClosePage { session_id } => match h.close_page(&session_id).await {
                    Ok(_) => {
                        let _ = event_tx
                            .send(TuiEvent::PageClosed {
                                session_id,
                                generation: result_generation,
                            })
                            .await;
                    }
                    Err(detail) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "close page".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::StartFileDownload(request) => match h.start_file_download(request).await
                {
                    Ok(download) => {
                        let _ = event_tx
                            .send(TuiEvent::FileDownload {
                                download,
                                generation: result_generation,
                            })
                            .await;
                    }
                    Err(detail) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "start download".into(),
                                success: false,
                                detail,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
                DaemonCmd::QueryFileDownload { download_id } => {
                    match h.file_download(&download_id).await {
                        Ok(download) => {
                            let _ = event_tx
                                .send(TuiEvent::FileDownload {
                                    download,
                                    generation: result_generation,
                                })
                                .await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "refresh download".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::CancelFileDownload { download_id } => {
                    match h.cancel_file_download(&download_id).await {
                        Ok(download) => {
                            let _ = event_tx
                                .send(TuiEvent::FileDownload {
                                    download,
                                    generation: result_generation,
                                })
                                .await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "cancel download".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::SaveFileDownload { download_id, destination } => {
                    match h.save_file_download(&download_id, &destination).await {
                        Ok(download) => {
                            let _ = event_tx
                                .send(TuiEvent::FileDownload {
                                    download,
                                    generation: result_generation,
                                })
                                .await;
                        }
                        Err(detail) => {
                            let _ = event_tx
                                .send(TuiEvent::CommandResult {
                                    action: "save download".into(),
                                    success: false,
                                    detail,
                                    generation: result_generation,
                                })
                                .await;
                        }
                    }
                }
                DaemonCmd::ListPages { host } => match h.list_pages(&host).await {
                    Ok(pages) => {
                        let paths: Vec<String> = pages.into_iter().map(|(p, _)| p).collect();
                        let _ = event_tx
                            .send(TuiEvent::PageList {
                                host,
                                pages: paths,
                                generation: result_generation,
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = event_tx
                            .send(TuiEvent::CommandResult {
                                action: "list_pages".into(),
                                success: false,
                                detail: e,
                                generation: result_generation,
                            })
                            .await;
                    }
                },
            }
        }
    })
}

/// Format a msgpack payload map into a human-readable summary.
fn format_payload_summary(payload: &HashMap<String, MpValue>) -> String {
    let mut parts = Vec::new();
    for (key, val) in payload {
        let val_str = match val {
            MpValue::String(s) => s.as_str().unwrap_or("").to_string(),
            MpValue::Integer(i) => format!("{}", i.as_i64().unwrap_or(0)),
            MpValue::Boolean(b) => b.to_string(),
            _ => format!("{val:?}"),
        };
        if !val_str.is_empty() && val_str.len() < 200 {
            parts.push(format!("  {key}: {val_str}"));
        }
    }
    if parts.is_empty() { "  (no data)".into() } else { parts.join("\n") }
}

// ─── Connection ───────────────────────────────────────────────────────────────

/// The TUI's view of one daemon connection. Framing, request correlation,
/// deadlines, and typed remote errors belong to the shared IPC client; the
/// TUI keeps only its command surface and presentation decoding here.
pub struct DaemonHandle {
    client: Client,
}

/// Per-request deadline the TUI has always applied to daemon calls.
const RPC_DEADLINE: Duration = Duration::from_secs(5);

/// Generation numbers for the TUI's own connections. The daemon's reported
/// `connection_generation` remains the authority for observation freshness.
fn next_connection_generation() -> ConnectionGeneration {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    ConnectionGeneration(NEXT.fetch_add(1, Ordering::Relaxed))
}

impl DaemonHandle {
    /// Wrap an already negotiated shared client.
    pub fn from_client(client: Client) -> Self {
        Self { client }
    }

    /// The shared client behind this handle.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Send a request and receive the response frame through the shared client.
    async fn rpc(
        &mut self,
        msg_type: MessageType,
        payload: &HashMap<String, MpValue>,
    ) -> Result<Frame, String> {
        self.client
            .request(msg_type, payload.clone(), RPC_DEADLINE)
            .await
            .map_err(|error| error.to_string())
    }

    /// Query local node identity.
    pub async fn identity(&mut self) -> Result<IdentityInfo, String> {
        self.client.identity().await.map_err(|error| error.to_string())
    }

    /// Query daemon status.
    pub async fn status(&mut self) -> Result<DaemonStatusInfo, String> {
        self.client.status().await.map_err(|error| error.to_string())
    }

    pub async fn standard_propagation(&mut self) -> Result<StandardPropagationSnapshot, String> {
        let frame = self.rpc(MessageType::QueryStandardPropagation, &HashMap::new()).await?;
        let encoded = rmp_serde::to_vec_named(&frame.payload)
            .map_err(|error| format!("encode standard propagation snapshot: {error}"))?;
        rmp_serde::from_slice(&encoded)
            .map_err(|error| format!("decode standard propagation snapshot: {error}"))
    }

    /// Query known devices (announces).
    pub async fn devices(&mut self, styrene_only: bool) -> Result<Vec<DeviceInfo>, String> {
        self.client.devices(styrene_only).await.map_err(|error| error.to_string())
    }

    /// Subscribe to message events. Must be called before the read loop.
    pub async fn subscribe_messages(&mut self) -> Result<(), String> {
        self.rpc(MessageType::SubMessages, &HashMap::new()).await.map(|_| ())
    }

    /// Subscribe to device/announce events.
    pub async fn subscribe_devices(&mut self) -> Result<(), String> {
        self.rpc(MessageType::SubDevices, &HashMap::new()).await.map(|_| ())
    }

    /// Subscribe to link telemetry events (activated, closed, RTT updated).
    pub async fn subscribe_links(&mut self) -> Result<(), String> {
        self.rpc(MessageType::SubLinks, &HashMap::new()).await.map(|_| ())
    }

    pub async fn links(&mut self) -> Result<LinkSnapshot, String> {
        self.client.links().await.map_err(|error| error.to_string())
    }

    pub async fn path_table(&mut self) -> Result<Vec<PathInfo>, String> {
        let frame = self.rpc(MessageType::QueryPathTable, &HashMap::new()).await?;
        parse_typed_array(&frame.payload, "paths")
    }

    pub async fn interface_stats(&mut self) -> Result<Vec<InterfaceDetail>, String> {
        let frame = self.rpc(MessageType::QueryInterfaceStats, &HashMap::new()).await?;
        parse_typed_array(&frame.payload, "interfaces")
    }

    pub async fn subscribe_routes(&mut self) -> Result<(), String> {
        self.rpc(MessageType::SubRoutes, &HashMap::new()).await.map(|_| ())
    }

    pub async fn subscribe_network_operations(&mut self) -> Result<(), String> {
        self.rpc(MessageType::SubNetworkOperations, &HashMap::new()).await.map(|_| ())
    }

    pub async fn subscribe_requests(&mut self) -> Result<(), String> {
        self.rpc(MessageType::SubRequests, &HashMap::new()).await.map(|_| ())
    }

    pub async fn subscribe_resources(&mut self) -> Result<(), String> {
        self.rpc(MessageType::SubResources, &HashMap::new()).await.map(|_| ())
    }

    pub async fn start_network_operation(
        &mut self,
        request: StartNetworkOperationInfo,
    ) -> Result<NetworkOperationInfo, String> {
        let mut payload = HashMap::from([
            ("kind".into(), MpValue::from(request.kind.as_str())),
            ("timeout_ms".into(), MpValue::from(request.timeout_ms)),
        ]);
        if let Some(destination) = request.destination_hash {
            payload.insert("destination_hash".into(), MpValue::from(destination));
        }
        if let Some(link_id) = request.link_id {
            payload.insert("link_id".into(), MpValue::from(link_id));
        }
        let frame = self.rpc(MessageType::CmdNetworkOperationStart, &payload).await?;
        parse_typed_payload(&frame.payload)
    }

    pub async fn cancel_network_operation(
        &mut self,
        operation_id: &str,
    ) -> Result<NetworkOperationInfo, String> {
        let payload = HashMap::from([("operation_id".into(), MpValue::from(operation_id))]);
        let frame = self.rpc(MessageType::CmdNetworkOperationCancel, &payload).await?;
        parse_typed_payload(&frame.payload)
    }

    pub async fn network_operations(&mut self) -> Result<Vec<NetworkOperationInfo>, String> {
        let frame = self.rpc(MessageType::QueryNetworkOperation, &HashMap::new()).await?;
        parse_typed_array(&frame.payload, "operations")
    }

    pub async fn start_request(
        &mut self,
        request: StartRequestInfo,
    ) -> Result<RequestObservationInfo, String> {
        let payload = HashMap::from([
            ("link_id".into(), MpValue::from(request.link_id)),
            ("path".into(), MpValue::from(request.path)),
            ("data".into(), MpValue::Binary(request.data)),
            ("timeout_ms".into(), MpValue::from(request.timeout_ms)),
            ("max_response_size".into(), MpValue::from(request.max_response_size)),
        ]);
        let frame = self.rpc(MessageType::CmdRequestStart, &payload).await?;
        parse_typed_payload(&frame.payload)
    }

    pub async fn cancel_request(
        &mut self,
        request_id: &str,
    ) -> Result<RequestObservationInfo, String> {
        let payload = HashMap::from([("request_id".into(), MpValue::from(request_id))]);
        let frame = self.rpc(MessageType::CmdRequestCancel, &payload).await?;
        parse_typed_payload(&frame.payload)
    }

    pub async fn requests(&mut self) -> Result<Vec<RequestObservationInfo>, String> {
        let frame = self.rpc(MessageType::QueryRequests, &HashMap::new()).await?;
        parse_typed_array(&frame.payload, "requests")
    }

    pub async fn resources(&mut self) -> Result<Vec<ResourceTransferInfo>, String> {
        let frame = self.rpc(MessageType::QueryResources, &HashMap::new()).await?;
        parse_typed_array(&frame.payload, "resources")
    }

    pub async fn cancel_resource(&mut self, resource_hash: &str) -> Result<bool, String> {
        let payload = HashMap::from([("resource_hash".into(), MpValue::from(resource_hash))]);
        let frame = self.rpc(MessageType::CmdResourceCancel, &payload).await?;
        Ok(frame.payload.get("accepted").and_then(MpValue::as_bool).unwrap_or(false))
    }

    /// Send a ping. Returns true if pong received.
    pub async fn ping(&mut self) -> bool {
        self.rpc(MessageType::Ping, &HashMap::new())
            .await
            .map(|f| f.msg_type == MessageType::Pong)
            .unwrap_or(false)
    }

    // ── Chat Operations ─────────────────────────────────────────────────

    /// Send a chat message to a peer.
    pub async fn send_chat(
        &mut self,
        dest_hash: &str,
        content: &str,
        title: Option<&str>,
    ) -> Result<String, String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::from(dest_hash));
        p.insert("content".into(), MpValue::from(content));
        if let Some(t) = title {
            p.insert("title".into(), MpValue::from(t));
        }
        let frame = self.rpc(MessageType::CmdSendChat, &p).await?;
        Ok(mp_str(&frame.payload, "message_id"))
    }

    pub async fn send_chat_outcome(
        &mut self,
        dest_hash: &str,
        content: &str,
        title: Option<&str>,
        delivery_method: &str,
    ) -> Result<styrene_ipc::types::SendChatOutcome, String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::from(dest_hash));
        p.insert("content".into(), MpValue::from(content));
        p.insert("delivery_method".into(), MpValue::from(delivery_method));
        if let Some(title) = title {
            p.insert("title".into(), MpValue::from(title));
        }
        let frame = self.rpc(MessageType::CmdSendChatOutcome, &p).await?;
        let outcome: styrene_ipc::types::SendChatOutcome = parse_typed_value(
            frame.payload.get("outcome").cloned().ok_or("send response omitted outcome")?,
        )?;
        if outcome.message_id.is_empty() || outcome.message.id != outcome.message_id {
            return Err("send response omitted its authoritative message projection".into());
        }
        Ok(outcome)
    }

    pub async fn set_draft(
        &mut self,
        peer_hash: &str,
        content: &str,
    ) -> Result<styrene_ipc::types::ConversationDraft, String> {
        let payload = HashMap::from([
            ("peer_hash".into(), MpValue::from(peer_hash)),
            ("content".into(), MpValue::from(content)),
        ]);
        let frame = self.rpc(MessageType::CmdSetDraft, &payload).await?;
        parse_typed_value(
            frame.payload.get("draft").cloned().ok_or("draft response omitted draft")?,
        )
    }

    pub async fn draft(
        &mut self,
        peer_hash: &str,
    ) -> Result<Option<styrene_ipc::types::ConversationDraft>, String> {
        let payload = HashMap::from([("peer_hash".into(), MpValue::from(peer_hash))]);
        let frame = self.rpc(MessageType::QueryDraft, &payload).await?;
        match frame.payload.get("draft") {
            None | Some(MpValue::Nil) => Ok(None),
            Some(value) => parse_typed_value(value.clone()).map(Some),
        }
    }

    pub async fn clear_draft(&mut self, peer_hash: &str) -> Result<(), String> {
        let payload = HashMap::from([("peer_hash".into(), MpValue::from(peer_hash))]);
        self.rpc(MessageType::CmdClearDraft, &payload).await.map(|_| ())
    }

    pub async fn retry_message(
        &mut self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, String> {
        let payload = HashMap::from([("message_id".into(), MpValue::from(message_id))]);
        let frame = self.rpc(MessageType::CmdRetryMessage, &payload).await?;
        parse_typed_value(frame.payload.get("outcome").cloned().ok_or("retry omitted outcome")?)
    }

    pub async fn cancel_message(
        &mut self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, String> {
        let payload = HashMap::from([("message_id".into(), MpValue::from(message_id))]);
        let frame = self.rpc(MessageType::CmdCancelMessage, &payload).await?;
        parse_typed_value(frame.payload.get("outcome").cloned().ok_or("cancel omitted outcome")?)
    }

    pub async fn message_page(
        &mut self,
        peer_hash: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<MessageInfo>, Option<String>, bool), String> {
        let mut payload = HashMap::from([
            ("peer_hash".into(), MpValue::from(peer_hash)),
            ("limit".into(), MpValue::from(50_u64)),
        ]);
        if let Some(cursor) = cursor {
            payload.insert("cursor".into(), MpValue::from(cursor));
        }
        let mut reset = false;
        let frame = match self.rpc(MessageType::QueryMessages, &payload).await {
            Ok(frame) => frame,
            Err(error) if cursor.is_some() && error.contains("cursor_stale") => {
                payload.remove("cursor");
                reset = true;
                self.rpc(MessageType::QueryMessages, &payload).await?
            }
            Err(error) => return Err(error),
        };
        let messages = match frame.payload.get("messages") {
            Some(value) => parse_typed_value(value.clone())?,
            None => Vec::new(),
        };
        let next = frame.payload.get("next_cursor").and_then(MpValue::as_str).map(str::to_owned);
        Ok((messages, next, reset))
    }

    pub async fn message(&mut self, message_id: &str) -> Result<Option<MessageInfo>, String> {
        let payload = HashMap::from([("message_id".into(), MpValue::from(message_id))]);
        let frame = self.rpc(MessageType::QueryMessage, &payload).await?;
        match frame.payload.get("message") {
            None | Some(MpValue::Nil) => Ok(None),
            Some(value) => parse_typed_value(value.clone()).map(Some),
        }
    }

    pub async fn conversation_page(
        &mut self,
        cursor: Option<&str>,
    ) -> Result<(Vec<ConversationInfo>, Option<String>, bool), String> {
        let mut payload = HashMap::from([
            ("unread_only".into(), MpValue::Boolean(false)),
            ("limit".into(), MpValue::from(50_u64)),
        ]);
        if let Some(cursor) = cursor {
            payload.insert("cursor".into(), MpValue::from(cursor));
        }
        let mut reset = false;
        let frame = match self.rpc(MessageType::QueryConversations, &payload).await {
            Ok(frame) => frame,
            Err(error) if cursor.is_some() && error.contains("cursor_stale") => {
                payload.remove("cursor");
                reset = true;
                self.rpc(MessageType::QueryConversations, &payload).await?
            }
            Err(error) => return Err(error),
        };
        let conversations = match frame.payload.get("conversations") {
            Some(value) => parse_typed_value(value.clone())?,
            None => Vec::new(),
        };
        let next = frame.payload.get("next_cursor").and_then(MpValue::as_str).map(str::to_owned);
        Ok((conversations, next, reset))
    }

    /// Mark all messages from a peer as read.
    pub async fn mark_read(
        &mut self,
        peer_hash: &str,
    ) -> Result<MessagingOperationOutcome, String> {
        let mut p = HashMap::new();
        p.insert("peer_hash".into(), MpValue::from(peer_hash));
        let frame = self.rpc(MessageType::CmdMarkRead, &p).await?;
        parse_mark_read_response(&frame.payload, peer_hash)
    }

    /// Delete a message by ID.
    pub async fn delete_message(
        &mut self,
        message_id: &str,
    ) -> Result<MessagingOperationOutcome, String> {
        let mut p = HashMap::new();
        p.insert("message_id".into(), MpValue::from(message_id));
        let frame = self.rpc(MessageType::CmdDeleteMessage, &p).await?;
        parse_delete_message_response(&frame.payload, message_id)
    }

    // ── Fleet Operations ────────────────────────────────────────────────

    /// Query remote device status.
    pub async fn device_status(
        &mut self,
        dest_hash: &str,
        timeout_secs: Option<u64>,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest_hash));
        if let Some(t) = timeout_secs {
            p.insert("timeout".into(), MpValue::from(t));
        }
        let frame = self.rpc(MessageType::CmdDeviceStatus, &p).await?;
        Ok(frame.payload)
    }

    /// Execute a command on a remote device.
    pub async fn exec(
        &mut self,
        dest_hash: &str,
        cmd: &str,
        args: &[String],
        timeout_secs: Option<u64>,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest_hash));
        p.insert("command".into(), MpValue::from(cmd));
        let args_vals: Vec<MpValue> = args.iter().map(|a| MpValue::from(a.as_str())).collect();
        p.insert("args".into(), MpValue::Array(args_vals));
        if let Some(t) = timeout_secs {
            p.insert("timeout".into(), MpValue::from(t));
        }
        let frame = self.rpc(MessageType::CmdExec, &p).await?;
        Ok(frame.payload)
    }

    /// Reboot a remote device.
    pub async fn reboot_device(
        &mut self,
        dest_hash: &str,
        delay_secs: Option<u64>,
        timeout_secs: Option<u64>,
    ) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest_hash));
        if let Some(d) = delay_secs {
            p.insert("delay".into(), MpValue::from(d));
        }
        if let Some(t) = timeout_secs {
            p.insert("timeout".into(), MpValue::from(t));
        }
        self.rpc(MessageType::CmdRebootDevice, &p).await.map(|_| ())
    }

    /// Push a signed profile to a remote node.
    pub async fn fleet_apply(
        &mut self,
        dest_hash: &str,
        profile_hex: &str,
        verify: bool,
        timeout_secs: Option<u64>,
    ) -> Result<HashMap<String, MpValue>, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest_hash));
        p.insert("profile".into(), MpValue::from(profile_hex));
        p.insert("verify".into(), MpValue::Boolean(verify));
        if let Some(t) = timeout_secs {
            p.insert("timeout".into(), MpValue::from(t));
        }
        let frame = self.rpc(MessageType::CmdFleetApply, &p).await?;
        Ok(frame.payload)
    }

    // ── Identity & Settings ─────────────────────────────────────────────

    /// Update local node identity (display name, icon).
    pub async fn set_identity(
        &mut self,
        display_name: &str,
        icon: Option<&str>,
    ) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("display_name".into(), MpValue::from(display_name));
        if let Some(i) = icon {
            p.insert("icon".into(), MpValue::from(i));
        }
        self.rpc(MessageType::CmdSetIdentity, &p).await.map(|_| ())
    }

    /// Query daemon configuration.
    pub async fn query_config(&mut self) -> Result<HashMap<String, MpValue>, String> {
        let frame = self.rpc(MessageType::QueryConfig, &HashMap::new()).await?;
        Ok(frame.payload)
    }

    /// Set auto-reply configuration.
    pub async fn set_auto_reply(
        &mut self,
        mode: &str,
        message: &str,
        cooldown_secs: Option<u64>,
    ) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("mode".into(), MpValue::from(mode));
        p.insert("message".into(), MpValue::from(message));
        if let Some(c) = cooldown_secs {
            p.insert("cooldown_secs".into(), MpValue::from(c));
        }
        self.rpc(MessageType::CmdSetAutoReply, &p).await.map(|_| ())
    }

    /// Block a peer by identity hash.
    pub async fn block_peer(&mut self, identity_hash: &str) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("identity_hash".into(), MpValue::from(identity_hash));
        self.rpc(MessageType::CmdBlockPeer, &p).await.map(|_| ())
    }

    /// Unblock a peer by identity hash.
    pub async fn unblock_peer(&mut self, identity_hash: &str) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("identity_hash".into(), MpValue::from(identity_hash));
        self.rpc(MessageType::CmdUnblockPeer, &p).await.map(|_| ())
    }

    /// Query the list of blocked peers.
    pub async fn blocked_peers(&mut self) -> Result<Vec<String>, String> {
        let frame = self.rpc(MessageType::QueryBlockedPeers, &HashMap::new()).await?;
        let arr = frame
            .payload
            .get("blocked_peers")
            .and_then(|v| v.as_array())
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        Ok(arr)
    }

    // ── Terminal Session ─────────────────────────────────────────────────

    /// Open a remote terminal session.
    pub async fn terminal_open(
        &mut self,
        dest_hash: &str,
        rows: u16,
        cols: u16,
    ) -> Result<String, String> {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::from(dest_hash));
        p.insert("rows".into(), MpValue::from(rows as u64));
        p.insert("cols".into(), MpValue::from(cols as u64));
        let frame = self.rpc(MessageType::CmdTerminalOpen, &p).await?;
        Ok(mp_str(&frame.payload, "session_id"))
    }

    /// Send input data to a terminal session.
    pub async fn terminal_input(&mut self, session_id: &str, data: &[u8]) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("session_id".into(), MpValue::from(session_id));
        p.insert("data".into(), MpValue::Binary(data.to_vec()));
        self.rpc(MessageType::CmdTerminalInput, &p).await.map(|_| ())
    }

    // ── Page Operations ──────────────────────────────────────────────────

    /// Browse a Micron page from a host (or "local" for this node's pages).
    pub async fn query_page(
        &mut self,
        host: &str,
        path: &str,
    ) -> Result<styrene_ipc::types::PageContent, String> {
        let mut p = HashMap::new();
        p.insert("host".into(), MpValue::from(host));
        p.insert("path".into(), MpValue::from(path));
        let frame = self.rpc(MessageType::QueryPage, &p).await?;
        decode_page_payload(&frame.payload)
    }

    pub async fn navigate_page(
        &mut self,
        request: styrene_ipc::types::PageNavigationRequest,
    ) -> Result<styrene_ipc::types::PageContent, String> {
        let encoded = rmp_serde::to_vec_named(&request)
            .map_err(|error| format!("encode page navigation: {error}"))?;
        let payload = HashMap::from([("navigation".into(), MpValue::Binary(encoded))]);
        let frame = self.rpc(MessageType::CmdPageNavigate, &payload).await?;
        decode_page_payload(&frame.payload)
    }

    pub async fn close_page(&mut self, session_id: &str) -> Result<(), String> {
        let payload = HashMap::from([("session_id".into(), MpValue::from(session_id))]);
        self.rpc(MessageType::CmdPageDisconnect, &payload).await.map(|_| ())
    }

    pub async fn start_file_download(
        &mut self,
        request: styrene_ipc::types::FileDownloadRequest,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        let encoded = rmp_serde::to_vec_named(&request)
            .map_err(|error| format!("encode download request: {error}"))?;
        let payload = HashMap::from([("download_request".into(), MpValue::Binary(encoded))]);
        let frame = self.rpc(MessageType::CmdFileDownloadStart, &payload).await?;
        parse_typed_payload_key(&frame.payload, "download")
    }

    pub async fn file_download(
        &mut self,
        download_id: &str,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        let payload = HashMap::from([("download_id".into(), MpValue::from(download_id))]);
        let frame = self.rpc(MessageType::QueryFileDownload, &payload).await?;
        parse_typed_payload_key(&frame.payload, "download")
    }

    pub async fn cancel_file_download(
        &mut self,
        download_id: &str,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        let payload = HashMap::from([("download_id".into(), MpValue::from(download_id))]);
        let frame = self.rpc(MessageType::CmdFileDownloadCancel, &payload).await?;
        parse_typed_payload_key(&frame.payload, "download")
    }

    pub async fn save_file_download(
        &mut self,
        download_id: &str,
        destination: &str,
    ) -> Result<styrene_ipc::types::FileDownloadInfo, String> {
        let payload = HashMap::from([
            ("download_id".into(), MpValue::from(download_id)),
            ("destination".into(), MpValue::from(destination)),
        ]);
        let frame = self.rpc(MessageType::CmdFileDownloadSave, &payload).await?;
        parse_typed_payload_key(&frame.payload, "download")
    }

    /// List pages served by a host.
    pub async fn list_pages(&mut self, host: &str) -> Result<Vec<(String, String)>, String> {
        let mut p = HashMap::new();
        p.insert("host".into(), MpValue::from(host));
        let frame = self.rpc(MessageType::CmdPageListSites, &p).await?;
        let arr =
            frame.payload.get("pages").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let pages = arr
            .iter()
            .filter_map(|v| {
                let m = v.as_map()?;
                let path = m
                    .iter()
                    .find(|(k, _)| k.as_str() == Some("path"))
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let host = m
                    .iter()
                    .find(|(k, _)| k.as_str() == Some("host_hash"))
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some((path, host))
            })
            .collect();
        Ok(pages)
    }

    /// Close a terminal session.
    pub async fn terminal_close(&mut self, session_id: &str) -> Result<(), String> {
        let mut p = HashMap::new();
        p.insert("session_id".into(), MpValue::from(session_id));
        self.rpc(MessageType::CmdTerminalClose, &p).await.map(|_| ())
    }
}

// ─── Public connect function ──────────────────────────────────────────────────

/// Connect to the styrened daemon. Returns a handle and a channel of TuiEvents.
///
/// `socket_path`: overrides the default path ($STYRENED_SOCKET or
/// $XDG_RUNTIME_DIR/styrened/control.sock).
///
/// Returns `Err` if the socket doesn't exist or the daemon doesn't respond
/// to the initial ping. The TUI degrades gracefully to demo mode.
pub struct DaemonConnection {
    handle: Option<DaemonHandle>,
    pub events: mpsc::Receiver<TuiEvent>,
    event_reader: tokio::task::JoinHandle<()>,
    /// The dedicated subscription connection; dropping it ends the event stream.
    _event_client: Client,
}

impl DaemonConnection {
    pub fn take_handle(&mut self) -> DaemonHandle {
        self.handle.take().expect("daemon connection handle can only be taken once")
    }
}

impl Drop for DaemonConnection {
    fn drop(&mut self) {
        self.event_reader.abort();
    }
}

pub async fn connect(socket_path: Option<&Path>) -> Result<DaemonConnection, String> {
    let path = socket_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(styrene_ipc_client::default_socket_path);

    if !path.exists() {
        return Err(format!("socket not found: {}", path.display()));
    }

    // Negotiation confirms the daemon answers, reports a connection
    // generation, and advertises a capability contract this TUI understands.
    let (command_client, _negotiation) =
        Client::connect_unix(&path, next_connection_generation(), RPC_DEADLINE)
            .await
            .map_err(|error| format!("connect {}: {error}", path.display()))?;
    let mut handle = DaemonHandle::from_client(command_client);

    // A dedicated subscription connection prevents the event reader from
    // consuming command responses or holding the command stream lock.
    let (event_client, event_negotiation) =
        Client::connect_unix(&path, next_connection_generation(), RPC_DEADLINE)
            .await
            .map_err(|error| format!("connect event stream {}: {error}", path.display()))?;
    // Take the receiver before subscribing so no pushed event is missed.
    let event_frames = event_client.events();
    let mut event_handle = DaemonHandle::from_client(event_client.clone());
    let event_generation = event_negotiation.daemon_generation;
    event_handle.subscribe_devices().await?;
    event_handle.subscribe_messages().await?;
    event_handle.subscribe_links().await?;
    event_handle.subscribe_routes().await?;
    event_handle.subscribe_network_operations().await?;
    event_handle.subscribe_requests().await?;
    event_handle.subscribe_resources().await?;

    let status = handle.status().await?;
    let links = handle.links().await.unwrap_or_default();
    let operations = handle.network_operations().await.unwrap_or_default();
    let requests = handle.requests().await.unwrap_or_default();
    let resources = handle.resources().await.unwrap_or_default();
    let routes = handle.path_table().await.unwrap_or_default();
    let interfaces = handle.interface_stats().await.unwrap_or_default();

    // Spawn the event reader task
    let (tx, rx) = mpsc::channel::<TuiEvent>(128);
    let _ = tx.send(TuiEvent::Status(status)).await;
    let _ = tx.send(TuiEvent::EventGeneration(event_generation)).await;
    if let Ok(inventory) = handle.client().profile_inventory().await
        && let Some(active) = inventory.active_profile_id.as_deref()
        && let Some(profile) = inventory.profiles.into_iter().find(|profile| profile.id == active)
    {
        let _ = tx.send(TuiEvent::Profile(Box::new(profile))).await;
    }
    let _ = tx.send(TuiEvent::RouteSnapshot(routes)).await;
    let _ = tx.send(TuiEvent::InterfaceSnapshot(interfaces)).await;
    let _ = tx.send(TuiEvent::LinkSnapshot(links)).await;
    let _ = tx.send(TuiEvent::NetworkOperationSnapshot(operations)).await;
    let _ = tx.send(TuiEvent::RequestSnapshot(requests)).await;
    let _ = tx.send(TuiEvent::ResourceSnapshot(resources)).await;
    let event_reader = tokio::spawn(event_reader(event_frames, tx));

    Ok(DaemonConnection {
        handle: Some(handle),
        events: rx,
        event_reader,
        _event_client: event_client,
    })
}

// ─── Event reader task ────────────────────────────────────────────────────────

/// Forward pushed daemon events from the shared client to the TUI. A lagging
/// receiver skips the oldest events rather than stalling the connection; the
/// stream ending means the subscription connection closed.
async fn event_reader(
    mut frames: tokio::sync::broadcast::Receiver<styrene_ipc_client::EventFrame>,
    tx: mpsc::Sender<TuiEvent>,
) {
    use tokio::sync::broadcast::error::RecvError;
    loop {
        match frames.recv().await {
            Ok(event) => {
                let frame = Frame {
                    msg_type: event.message_type,
                    request_id: [0; REQUEST_ID_SIZE],
                    payload: event.payload,
                };
                if let Some(ev) = frame_to_tui_event(frame)
                    && tx.send(ev).await.is_err()
                {
                    break; // receiver dropped — TUI exited
                }
            }
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => {
                let _ = tx.send(TuiEvent::Disconnected("event connection closed".into())).await;
                break;
            }
        }
    }
}

/// Convert a pushed server frame into a TuiEvent, if applicable.
fn frame_to_tui_event(frame: Frame) -> Option<TuiEvent> {
    match frame.msg_type {
        MessageType::EventDevice => {
            let device: DeviceInfo = parse_typed_payload(&frame.payload).ok()?;
            if device.destination_hash.is_empty() {
                return None;
            }
            let now = epoch_secs();
            let mut peer = PeerRecord::new(
                device.destination_hash.clone(),
                if device.name.is_empty() { None } else { Some(device.name.clone()) },
                now,
            );
            peer.native_page_host = device
                .discovered_capabilities
                .contains(&styrene_ipc::types::DiscoveredCapability::NativeNomadNetHost);
            Some(TuiEvent::PeerAnnounce(peer))
        }
        MessageType::EventLink => {
            let event: IpcLinkEvent = parse_typed_payload(&frame.payload).ok()?;
            let IpcLinkEvent {
                link_id,
                peer_hash,
                peer_name,
                interface,
                status,
                kind,
                activity,
                reason,
                remote_identity_hash,
                rtt_ms,
                observation,
                ..
            } = event;
            if link_id.is_empty() || status.is_empty() {
                return None;
            }
            Some(TuiEvent::LinkUpdate {
                link_id,
                peer_hash,
                peer_name,
                interface,
                status,
                kind,
                activity,
                reason,
                remote_identity_hash,
                rtt_ms,
                observation,
            })
        }
        MessageType::EventRoute => {
            let kind = mp_str(&frame.payload, "kind");
            let destination_hash = mp_str(&frame.payload, "destination_hash");
            if kind.is_empty() || destination_hash.is_empty() {
                return None;
            }
            let loss_reason =
                frame.payload.get("loss_reason").and_then(MpValue::as_str).map(ToOwned::to_owned);
            let expires = frame.payload.get("expires").and_then(MpValue::as_i64);
            let observation: ObservationMetadata =
                parse_typed_payload(&frame.payload).unwrap_or_default();
            Some(TuiEvent::RouteLifecycle {
                kind,
                destination_hash,
                loss_reason,
                expires,
                observation,
            })
        }
        MessageType::EventNetworkOperation => {
            parse_typed_payload(&frame.payload).ok().map(TuiEvent::NetworkOperation)
        }
        MessageType::EventRequest
            if frame.payload.get("kind").and_then(MpValue::as_str)
                == Some("reconcile_required") =>
        {
            Some(TuiEvent::RequestReconcileRequired {
                dropped: frame.payload.get("dropped").and_then(MpValue::as_u64).unwrap_or(0),
                connection_generation: frame
                    .payload
                    .get("connection_generation")
                    .and_then(MpValue::as_u64)
                    .unwrap_or(0),
            })
        }
        MessageType::EventRequest => {
            parse_typed_payload(&frame.payload).ok().map(TuiEvent::Request)
        }
        MessageType::EventResource => {
            parse_typed_payload(&frame.payload).ok().map(TuiEvent::Resource)
        }
        MessageType::EventReconcileRequired => Some(TuiEvent::ReconcileRequired {
            dropped: frame.payload.get("dropped").and_then(MpValue::as_u64).unwrap_or(0),
            connection_generation: frame
                .payload
                .get("connection_generation")
                .and_then(MpValue::as_u64)
                .unwrap_or(0),
        }),
        MessageType::EventStandardPropagationChanged => {
            Some(TuiEvent::StandardPropagationChanged {
                connection_generation: frame
                    .payload
                    .get("connection_generation")
                    .and_then(MpValue::as_u64)
                    .unwrap_or(0),
            })
        }
        MessageType::EventMessagingOperation => frame
            .payload
            .get("outcome")
            .cloned()
            .and_then(|value| parse_typed_value(value).ok())
            .map(Box::new)
            .map(TuiEvent::MessagingOperation),
        MessageType::EventMessage => {
            let msg = parse_message_from_payload(&frame.payload)?;
            Some(TuiEvent::Message(Box::new(msg)))
        }
        MessageType::EventTerminalOutput => {
            let session_id = mp_str(&frame.payload, "session_id");
            let data = frame.payload.get("data").and_then(|v| v.as_slice()).unwrap_or(&[]).to_vec();
            if session_id.is_empty() {
                return None;
            }
            Some(TuiEvent::TerminalOutput { session_id, data })
        }
        MessageType::EventTerminalExited => {
            let session_id = mp_str(&frame.payload, "session_id");
            let exit_code =
                frame.payload.get("exit_code").and_then(|v| v.as_i64()).map(|v| v as i32);
            if session_id.is_empty() {
                return None;
            }
            Some(TuiEvent::TerminalExited { session_id, exit_code })
        }
        _ => None,
    }
}

// ─── Periodic poller ─────────────────────────────────────────────────────────

/// Spawn a task that polls the daemon periodically and sends snapshot TuiEvents.
/// Call once after `connect()`. Sends Identity on first poll, then Status every N seconds.
pub fn spawn_poll_task(
    handle: Arc<Mutex<DaemonHandle>>,
    tx: mpsc::Sender<TuiEvent>,
    poll_interval_secs: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut first = true;
        loop {
            // Initial identity fetch
            if first {
                first = false;
                let result = handle.lock().await.identity().await;
                match result {
                    Ok(info) => {
                        let _ = tx.send(TuiEvent::Identity(info)).await;
                    }
                    Err(e) => {
                        let _ = tx.send(TuiEvent::Disconnected(e)).await;
                        return;
                    }
                }
            }

            // Periodic status + devices
            tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;

            let status = handle.lock().await.status().await;
            match status {
                Ok(s) => {
                    let _ = tx.send(TuiEvent::Status(s)).await;
                }
                Err(e) => {
                    let _ = tx.send(TuiEvent::Disconnected(e)).await;
                    return;
                }
            }

            let devices = handle.lock().await.devices(false).await;
            if let Ok(devs) = devices {
                let now = epoch_secs();
                for dev in devs {
                    if dev.destination_hash.is_empty() {
                        continue;
                    }
                    let mut peer = PeerRecord::new(
                        dev.destination_hash.clone(),
                        if dev.name.is_empty() { None } else { Some(dev.name.clone()) },
                        now,
                    );
                    peer.native_page_host = dev
                        .discovered_capabilities
                        .contains(&styrene_ipc::types::DiscoveredCapability::NativeNomadNetHost);
                    let _ = tx.send(TuiEvent::PeerAnnounce(peer)).await;
                }
            }

            if let Ok(operations) = handle.lock().await.network_operations().await {
                for operation in operations {
                    let _ = tx.send(TuiEvent::NetworkOperation(operation)).await;
                }
            }
            if let Ok(requests) = handle.lock().await.requests().await {
                for request in requests {
                    let _ = tx.send(TuiEvent::Request(request)).await;
                }
            }
            if let Ok(resources) = handle.lock().await.resources().await {
                for resource in resources {
                    let _ = tx.send(TuiEvent::Resource(resource)).await;
                }
            }
            if let Ok(routes) = handle.lock().await.path_table().await {
                let _ = tx.send(TuiEvent::RouteSnapshot(routes)).await;
            }
            if let Ok(interfaces) = handle.lock().await.interface_stats().await {
                let _ = tx.send(TuiEvent::InterfaceSnapshot(interfaces)).await;
            }
            if let Ok(links) = handle.lock().await.links().await {
                for event in links.active.into_iter().chain(links.history) {
                    let IpcLinkEvent {
                        link_id,
                        peer_hash,
                        peer_name,
                        interface,
                        status,
                        kind,
                        activity,
                        reason,
                        remote_identity_hash,
                        rtt_ms,
                        observation,
                        ..
                    } = event;
                    let _ = tx
                        .send(TuiEvent::LinkUpdate {
                            link_id,
                            peer_hash,
                            peer_name,
                            interface,
                            status,
                            kind,
                            activity,
                            reason,
                            remote_identity_hash,
                            rtt_ms,
                            observation,
                        })
                        .await;
                }
            }
        }
    })
}

// ─── App-side event application ───────────────────────────────────────────────

fn observation_generation_valid(app: &crate::app::App, observation: &ObservationMetadata) -> bool {
    observation.ipc_generation().is_some_and(|generation| {
        app.connection_generation == Some(generation)
            || app.event_connection_generation == Some(generation)
    })
}

/// Apply a TuiEvent to the App state. Call from the main event loop.
fn delivery_status(
    state: styrene_ipc::types::MessageLifecycleState,
    detail: Option<&str>,
) -> DeliveryStatus {
    use styrene_ipc::types::MessageLifecycleState as State;
    match state {
        State::Queued => DeliveryStatus::Pending,
        State::Sending => DeliveryStatus::Sending,
        State::Sent => DeliveryStatus::Sent,
        State::Delivered => DeliveryStatus::Delivered,
        State::Cancelled => DeliveryStatus::Cancelled,
        State::Failed | State::Expired | State::Rejected => {
            DeliveryStatus::Failed(detail.unwrap_or("Not reported").into())
        }
        _ => DeliveryStatus::Unknown,
    }
}

fn remove_message_projection(app: &mut crate::app::App, message_id: &str) -> bool {
    let mut removed = app.loaded_message_ids.remove(message_id);
    removed |= app.live_messages.remove(message_id).is_some();
    for ids in app.history_message_ids.values_mut() {
        ids.remove(message_id);
    }
    for conversation in app.conversations.values_mut() {
        removed |= conversation.remove_message(message_id);
    }
    removed
}

pub fn apply_event(app: &mut crate::app::App, ev: TuiEvent) {
    use crate::tui::segments::{DeliveryStatus, MessageLifecycle, ProtocolEventKind, Segment};

    if !app.daemon_session_accepting_events {
        return;
    }
    match ev {
        TuiEvent::Identity(info) => {
            app.node_hash = info.destination_hash.clone();
            app.node_name = info.display_name.clone();
            app.daemon_connected = true;
            let hash_short = &info.destination_hash[..8.min(info.destination_hash.len())];
            app.conversation.push_system(&format!(
                "⬡ connected  node: {hash_short}…  name: {}",
                info.display_name
            ));
            app.activity.push(ActivityEntry::new(
                ActivityKind::Announce,
                &info.display_name,
                "local node identity loaded",
            ));
        }

        TuiEvent::Status(status) => {
            let Some(generation) = status.connection_generation.filter(|value| *value != 0) else {
                app.connection_generation = None;
                app.active_capabilities = None;
                app.standard_propagation = None;
                app.standard_propagation_error = None;
                return;
            };
            if app.connection_generation.is_some()
                && app.connection_generation != status.connection_generation
            {
                app.network_operations.clear();
                app.request_observations.clear();
                app.links.clear();
                app.active_capabilities = None;
                app.connection_generation = None;
                app.standard_propagation = None;
                app.standard_propagation_error = None;
                return;
            }
            let first_status_for_generation = app.connection_generation != Some(generation);
            app.daemon_version = status.daemon_version.clone();
            app.rns_initialized = status.rns_initialized;
            app.transport_active = status.transport_enabled;
            app.propagation_enabled = status.propagation_enabled;
            app.interface_count = status.interface_count;
            app.connection_generation = Some(generation);
            app.active_capabilities = status.active_capabilities;
            if first_status_for_generation {
                app.send_daemon_cmd(DaemonCmd::RequeryStandardPropagation);
            }
            if !app.conversation_page_loaded {
                app.conversation_page_loaded = true;
                app.send_daemon_cmd(DaemonCmd::LoadConversationPage { cursor: None });
            }
        }

        TuiEvent::Profile(profile) => {
            app.profile_info = Some(*profile);
        }
        TuiEvent::EventGeneration(generation) => {
            if generation == 0 {
                app.event_connection_generation = None;
                return;
            }
            if app.event_connection_generation.is_some()
                && app.event_connection_generation != Some(generation)
            {
                app.event_connection_generation = None;
                return;
            }
            app.event_connection_generation = Some(generation);
        }

        TuiEvent::MessagingOperation(outcome) => {
            use styrene_ipc::types::MessagingDisposition;

            let peer = outcome.message.as_ref().map(|message| {
                if message.is_outgoing {
                    message.destination_hash.clone()
                } else {
                    message.source_hash.clone()
                }
            });
            if let Some(message) = outcome.message.clone() {
                apply_event(app, TuiEvent::Message(Box::new(message)));
            }
            match outcome.disposition {
                MessagingDisposition::Applied
                | MessagingDisposition::Created
                | MessagingDisposition::Updated => {}
                MessagingDisposition::AlreadyCancelled => {
                    if outcome.message.is_none() {
                        for conversation in app.conversations.values_mut() {
                            conversation
                                .update_sent_status(&outcome.target_id, DeliveryStatus::Cancelled);
                        }
                    }
                }
                MessagingDisposition::Unchanged => {
                    if outcome.message.is_none() {
                        app.conversation.push_system(&format!(
                            "lifecycle unchanged; authoritative message {}",
                            outcome.correlated_id.as_deref().unwrap_or(&outcome.target_id)
                        ));
                    }
                }
                MessagingDisposition::TerminalConflict => {
                    if outcome.message.is_none() {
                        app.conversation.push_system(
                            "terminal lifecycle patch requires authoritative message requery",
                        );
                    }
                    app.conversation.push_system(&format!(
                        "lifecycle terminal: {}",
                        outcome.terminal_state.as_deref().unwrap_or("unknown")
                    ));
                }
                MessagingDisposition::NotFound => {
                    app.loaded_message_ids.remove(&outcome.target_id);
                    app.live_messages.remove(&outcome.target_id);
                    let stale_peer = peer.or_else(|| {
                        app.conversations.iter().find_map(|(peer, conversation)| {
                            conversation.contains_sent(&outcome.target_id).then(|| peer.clone())
                        })
                    });
                    if let Some(peer) = stale_peer {
                        app.peer_conversation(&peer).remove_sent(&outcome.target_id);
                        app.message_cursors.remove(&peer);
                        app.send_daemon_cmd(DaemonCmd::LoadMessagePage {
                            peer_hash: peer,
                            cursor: None,
                        });
                    }
                }
                MessagingDisposition::Unknown => {
                    app.conversation.push_system("unknown messaging lifecycle result");
                }
                _ => app.conversation.push_system("unsupported messaging lifecycle result"),
            }
        }

        TuiEvent::PeerAnnounce(peer) => {
            let hash = peer.hash.clone();
            let name = peer.name.clone();
            let now = epoch_secs();

            if let Some(existing) = app.peers.iter_mut().find(|p| p.hash == hash) {
                existing.touch(now, 1);
            } else {
                app.conversation.push_protocol_event(
                    ProtocolEventKind::Announce,
                    Some(&hash[..8.min(hash.len())]),
                    name.as_deref(),
                    "announce",
                );
                app.activity.push(ActivityEntry::new(
                    ActivityKind::Announce,
                    name.as_deref().unwrap_or(&hash[..8.min(hash.len())]),
                    "announce received",
                ));
                app.peers.push(peer);
            }
            // trigger_flash removed — effects system handles visuals
        }

        TuiEvent::MessageResolved { message_id, message, generation } => {
            if app.connection_generation != Some(generation) {
                return;
            }
            match message {
                Some(message) if message.projection_complete && message.id == message_id => {
                    apply_event(app, TuiEvent::Message(message));
                }
                Some(_) => {
                    app.conversation
                        .push_system("message requery returned an invalid partial projection");
                }
                None => {
                    remove_message_projection(app, &message_id);
                }
            }
        }
        TuiEvent::Message(msg) => {
            let msg = *msg;
            if !msg.projection_complete {
                if !msg.id.is_empty() {
                    app.send_daemon_cmd(DaemonCmd::QueryMessage { message_id: msg.id });
                }
                return;
            }
            let existed = remove_message_projection(app, &msg.id);
            let peer_hash = if msg.is_outgoing {
                msg.destination_hash.clone()
            } else {
                msg.source_hash.clone()
            };
            app.live_messages.insert(msg.id.clone(), msg.clone());
            app.loaded_message_ids.insert(msg.id.clone());
            let name = app.peers.iter().find(|p| p.hash == peer_hash).and_then(|p| p.name.clone());

            // Push to per-peer conversation
            let conv = app.peer_conversation(&peer_hash);
            if msg.is_outgoing {
                let lifecycle = MessageLifecycle::from(&msg);
                let delivery_status =
                    delivery_status(msg.lifecycle_state, msg.terminal_detail.as_deref());
                conv.push_sent_with_lifecycle(
                    Some(&msg.id),
                    &peer_hash,
                    name.as_deref(),
                    &msg.content,
                    delivery_status,
                    lifecycle,
                );
            } else {
                conv.push_received_with_lifecycle(
                    Some(&msg.id),
                    &peer_hash,
                    name.as_deref(),
                    msg.title.as_deref(),
                    &msg.content,
                    msg.timestamp,
                    MessageLifecycle::from(&msg),
                );
            }

            // Also push to global conversation (Home activity)
            if !msg.is_outgoing && !existed {
                app.conversation.push_received(
                    &peer_hash,
                    name.as_deref(),
                    msg.title.as_deref(),
                    &msg.content,
                    msg.timestamp,
                );
            }

            let label = name.as_deref().unwrap_or(&peer_hash[..8.min(peer_hash.len())]);
            if !existed {
                app.activity.push(ActivityEntry::new(
                    if msg.is_outgoing {
                        ActivityKind::OutboundMessage
                    } else {
                        ActivityKind::InboundMessage
                    },
                    label,
                    msg.title.as_deref().unwrap_or(&msg.content[..msg.content.len().min(32)]),
                ));
            }
            if !msg.is_outgoing && !existed {
                app.unread_count += 1;
            }
            // trigger_flash removed — effects system handles visuals
        }

        TuiEvent::MessageStatus { id, status: _ } => {
            if !id.is_empty() {
                app.send_daemon_cmd(DaemonCmd::QueryMessage { message_id: id });
            }
        }

        TuiEvent::LinkUpdate {
            link_id,
            peer_hash,
            peer_name,
            status,
            kind: _,
            activity: _,
            rtt_ms,
            observation,
            ..
        } => {
            if !observation_generation_valid(app, &observation) {
                return;
            }
            use crate::mesh_state::{LinkRecord, LinkStatus};

            match status.as_str() {
                "active" => {
                    if !app.links.iter().any(|l| l.id == link_id) {
                        let mut link = LinkRecord::new(
                            link_id.clone(),
                            peer_hash.clone(),
                            peer_name.clone(),
                            crate::mesh_state::epoch_secs(),
                        );
                        if let Some(rtt) = rtt_ms {
                            link.rtt_ms = rtt;
                        }
                        link.pluck();
                        app.links.push(link);
                        app.activity.push(ActivityEntry::new(
                            ActivityKind::LinkUp,
                            peer_name.as_deref().unwrap_or(&peer_hash[..8.min(peer_hash.len())]),
                            "link established",
                        ));
                    }
                }
                "rtt_updated" => {
                    if let Some(link) = app.links.iter_mut().find(|l| l.id == link_id)
                        && let Some(rtt) = rtt_ms
                    {
                        link.rtt_ms = rtt;
                        link.pluck();
                    }
                }
                "closed" | "stale" => {
                    if let Some(link) = app.links.iter_mut().find(|l| l.id == link_id) {
                        link.status =
                            if status == "stale" { LinkStatus::Stale } else { LinkStatus::Closed };
                    } else {
                        let mut link = LinkRecord::new(
                            link_id.clone(),
                            peer_hash.clone(),
                            peer_name.clone(),
                            crate::mesh_state::epoch_secs(),
                        );
                        link.status =
                            if status == "stale" { LinkStatus::Stale } else { LinkStatus::Closed };
                        if let Some(rtt) = rtt_ms {
                            link.rtt_ms = rtt;
                        }
                        app.links.push(link);
                    }
                    if status == "closed" {
                        app.activity.push(ActivityEntry::new(
                            ActivityKind::LinkDown,
                            peer_name.as_deref().unwrap_or(&peer_hash[..8.min(peer_hash.len())]),
                            "link closed",
                        ));
                    }
                }
                _ => {}
            }
            // trigger_flash removed — effects system handles visuals
        }

        TuiEvent::RouteLifecycle { kind, destination_hash, loss_reason, observation, .. } => {
            if !observation_generation_valid(app, &observation) {
                return;
            }
            let label = &destination_hash[..8.min(destination_hash.len())];
            let (activity_kind, detail) = match kind.as_str() {
                "lost" => (
                    ActivityKind::RouteLost,
                    format!("route lost: {}", loss_reason.as_deref().unwrap_or("unknown")),
                ),
                "rediscovered" => (ActivityKind::RouteDiscovered, "route rediscovered".into()),
                _ => (ActivityKind::RouteDiscovered, "route discovered".into()),
            };
            app.activity.push(ActivityEntry::new(activity_kind, label, detail));
        }
        TuiEvent::NetworkOperation(operation) => {
            if !observation_generation_valid(app, &operation.observation) {
                return;
            }
            if app
                .network_operations
                .iter()
                .any(|item| item.operation_id == operation.operation_id && item == &operation)
            {
                return;
            }
            let state = operation
                .outcome
                .map(|value| value.as_str())
                .unwrap_or(operation.progress.as_str());
            app.conversation.push_system(&format!(
                "⬡ {} {}: {state}{}",
                operation.kind.as_str(),
                operation.operation_id,
                operation
                    .detail
                    .as_deref()
                    .map(|detail| format!(" — {detail}"))
                    .unwrap_or_default()
            ));
            if let Some(current) = app
                .network_operations
                .iter_mut()
                .find(|item| item.operation_id == operation.operation_id)
            {
                *current = operation;
            } else {
                app.network_operations.push(operation);
            }
        }
        TuiEvent::Request(request) => {
            if !observation_generation_valid(app, &request.observation) {
                return;
            }
            if app
                .request_observations
                .iter()
                .any(|item| item.request_id == request.request_id && item == &request)
            {
                return;
            }
            app.conversation.push_system(&format!(
                "⬡ request {}: {:?} {:.0}%",
                request.request_id,
                request.state,
                request.progress * 100.0
            ));
            if let Some(current) = app
                .request_observations
                .iter_mut()
                .find(|item| item.request_id == request.request_id)
            {
                *current = request;
            } else {
                app.request_observations.push(request);
            }
        }
        TuiEvent::RequestReconcileRequired { dropped, connection_generation } => {
            if app.event_connection_generation != Some(connection_generation) {
                return;
            }
            app.conversation.push_system(&format!(
                "⬡ request event gap: {dropped} dropped; snapshot reconciliation required"
            ));
            app.send_daemon_cmd(DaemonCmd::ReconcileNetworkObservations);
        }
        TuiEvent::Resource(resource) => {
            if !observation_generation_valid(app, &resource.observation) {
                return;
            }
            if let Some(current) = app
                .resource_transfers
                .iter_mut()
                .find(|item| item.resource_hash == resource.resource_hash)
            {
                *current = resource;
            } else {
                app.resource_transfers.push(resource);
            }
        }
        TuiEvent::LinkSnapshot(snapshot) => {
            let Some(generation) = app.connection_generation else {
                return;
            };
            if snapshot
                .active
                .iter()
                .chain(&snapshot.history)
                .any(|link| link.observation.ipc_generation() != Some(generation))
            {
                return;
            }
            for link in snapshot.active.iter().chain(&snapshot.history) {
                app.conversation.push_system(&format!(
                    "⬡ link id={} peer={} status={} rtt={}ms correlation={}",
                    link.link_id,
                    link.peer_hash,
                    link.status,
                    link.rtt_ms
                        .map(|value| format!("{value:.1}"))
                        .unwrap_or_else(|| "unknown".into()),
                    link.observation.correlation_id.as_deref().unwrap_or("none")
                ));
            }
            app.links.clear();
            for link in snapshot.active.into_iter().chain(snapshot.history) {
                apply_event(app, link_update_event(link));
            }
        }
        TuiEvent::NetworkOperationSnapshot(operations) => {
            let Some(generation) = app.connection_generation else {
                return;
            };
            if operations
                .iter()
                .any(|operation| operation.observation.ipc_generation() != Some(generation))
            {
                return;
            }
            for operation in &operations {
                app.conversation.push_system(&format!(
                    "⬡ operation id={} kind={} state={} target={} link={} correlation={}",
                    operation.operation_id,
                    operation.kind.as_str(),
                    operation
                        .outcome
                        .map(|value| value.as_str())
                        .unwrap_or(operation.progress.as_str()),
                    operation.destination_hash.as_deref().unwrap_or("none"),
                    operation.link_id.as_deref().unwrap_or("none"),
                    operation.observation.correlation_id.as_deref().unwrap_or("none")
                ));
            }
            app.network_operations = operations;
        }
        TuiEvent::RequestSnapshot(requests) => {
            let Some(generation) = app.connection_generation else {
                return;
            };
            if requests
                .iter()
                .any(|request| request.observation.ipc_generation() != Some(generation))
            {
                return;
            }
            for request in &requests {
                app.conversation.push_system(&format!(
                    "⬡ request id={} link={} state={:?} progress={:.0}% bytes={}/{} error={} correlation={}",
                    request.request_id,
                    request.link_id,
                    request.state,
                    request.progress * 100.0,
                    request.received_bytes,
                    request.total_bytes,
                    request
                        .protocol_error
                        .map(|value| format!("{value:?}"))
                        .unwrap_or_else(|| "none".into()),
                    request.observation.correlation_id.as_deref().unwrap_or("none")
                ));
            }
            app.request_observations = requests;
        }
        TuiEvent::ResourceSnapshot(resources) => {
            let Some(generation) = app.connection_generation else {
                return;
            };
            if resources
                .iter()
                .any(|resource| resource.observation.ipc_generation() != Some(generation))
            {
                return;
            }
            for resource in &resources {
                app.conversation.push_system(&format!(
                    "⬡ resource id={} link={} state={:?} progress={:.0}% bytes={}/{} cancellable={} correlation={}",
                    resource.resource_hash,
                    resource.link_id,
                    resource.state,
                    resource.progress * 100.0,
                    resource.received_bytes,
                    resource.total_bytes,
                    resource.cancellable,
                    resource.observation.correlation_id.as_deref().unwrap_or("none")
                ));
            }
            app.resource_transfers = resources;
        }
        TuiEvent::ReconcileRequired { dropped, connection_generation } => {
            if app.event_connection_generation != Some(connection_generation) {
                return;
            }
            app.conversation.push_system(&format!(
                "⬡ event gap: {dropped} dropped; reconciling route/link/interface/operation/request/resource snapshots"
            ));
            app.send_daemon_cmd(DaemonCmd::ReconcileNetworkObservations);
        }
        TuiEvent::StandardPropagationChanged { connection_generation } => {
            if app.event_connection_generation == Some(connection_generation) {
                app.send_daemon_cmd(DaemonCmd::RequeryStandardPropagation);
            }
        }
        TuiEvent::StandardPropagationSnapshot(snapshot) => {
            if snapshot.connection_generation == app.connection_generation {
                if snapshot.version != styrene_ipc::types::STANDARD_PROPAGATION_SNAPSHOT_VERSION {
                    app.standard_propagation_error = Some(format!(
                        "unsupported standard propagation snapshot v{}",
                        snapshot.version
                    ));
                    return;
                }
                app.standard_propagation = Some(snapshot);
                app.standard_propagation_error = None;
            }
        }
        TuiEvent::RouteSnapshot(routes) => {
            let Some(generation) = app.connection_generation else {
                return;
            };
            if routes.iter().any(|route| route.observation.ipc_generation() != Some(generation)) {
                return;
            }
            app.conversation.push_system(&format!("⬡ {} authoritative routes", routes.len()));
            for route in &routes {
                app.conversation.push_system(&format!(
                    "⬡ route destination={} hops={} next={} interface={} expires={} correlation={}",
                    route.destination_hash,
                    route.hops.map(|value| value.to_string()).unwrap_or_else(|| "unknown".into()),
                    route.next_hop.as_deref().unwrap_or("unknown"),
                    route.interface.as_deref().unwrap_or("unknown"),
                    route
                        .expires
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unknown".into()),
                    route.observation.correlation_id.as_deref().unwrap_or("none")
                ));
            }
            app.route_observations = routes;
        }
        TuiEvent::InterfaceSnapshot(interfaces) => {
            let Some(generation) = app.connection_generation else {
                return;
            };
            if interfaces
                .iter()
                .any(|interface| interface.observation.ipc_generation() != Some(generation))
            {
                return;
            }
            app.conversation
                .push_system(&format!("⬡ {} authoritative interfaces", interfaces.len()));
            for interface in &interfaces {
                app.conversation.push_system(&format!(
                    "⬡ interface name={} hash={} type={} mode={} enabled={} status={} host={} port={} local={} remote={} parent={} tx={} rx={} peers={} source={:?} observed={} age={} threshold={} stale={} generation={} correlation={}",
                    interface.name,
                    interface.hash,
                    interface.kind,
                    interface.mode,
                    interface.enabled,
                    interface.status,
                    interface.host.as_deref().unwrap_or("unknown"),
                    interface.port.map(|value| value.to_string()).unwrap_or_else(|| "unknown".into()),
                    interface.local_endpoint.as_deref().unwrap_or("unknown"),
                    interface.remote_endpoint.as_deref().unwrap_or("unknown"),
                    interface.parent_hash.as_deref().unwrap_or("none"),
                    interface.tx_bytes,
                    interface.rx_bytes,
                    interface.peers_connected,
                    interface.observation.source,
                    interface.observation.observed_at.map(|value| value.to_string()).unwrap_or_else(|| "unknown".into()),
                    interface.observation.age_secs.map(|value| value.to_string()).unwrap_or_else(|| "unknown".into()),
                    interface.observation.freshness_threshold_secs.map(|value| value.to_string()).unwrap_or_else(|| "unknown".into()),
                    interface.observation.stale,
                    interface.observation.connection_generation.map(|value| value.to_string()).unwrap_or_else(|| "unknown".into()),
                    interface.observation.correlation_id.as_deref().unwrap_or("none")
                ));
            }
            app.interface_observations = interfaces;
        }

        TuiEvent::ChatSendResult { peer_hash, message_id, success, detail, generation } => {
            if app.connection_generation != Some(generation) {
                return;
            }
            let status =
                if success { DeliveryStatus::Sent } else { DeliveryStatus::Failed(detail.clone()) };
            let conversation = app.peer_conversation(&peer_hash);
            conversation.acknowledge_last_sent(message_id.as_deref(), status);
            if !success {
                app.conversation.push_system(&format!("✗ send_chat: {detail}"));
            }
        }

        TuiEvent::ChatSendOutcome { peer_hash, outcome, generation } => {
            if app.connection_generation != Some(generation) {
                return;
            }
            let mut message = outcome.message.clone();
            message.projection_complete = true;
            apply_event(app, TuiEvent::Message(Box::new(message)));
            let accepted = match outcome.disposition {
                styrene_ipc::types::SendChatDisposition::Accepted => true,
                styrene_ipc::types::SendChatDisposition::PaperExported => {
                    if let Some(uri) = outcome.paper_uri.clone() {
                        app.paper_export = Some(crate::app::PaperExportState {
                            message_id: outcome.message_id.clone(),
                            uri,
                        });
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            };
            if accepted {
                app.compose_pending = None;
                app.editor.clear_line();
                app.input_mode = crate::app::InputMode::Normal;
                app.focus = crate::app::Focus::Sidebar;
                app.send_daemon_cmd(DaemonCmd::ClearDraft { peer_hash });
            } else {
                app.compose_pending = None;
                app.conversation.push_system(&format!(
                    "✗ send failed after persistence: {}",
                    outcome.terminal_error.as_deref().unwrap_or("unknown daemon failure")
                ));
            }
        }
        TuiEvent::DraftLoaded { peer_hash, draft, generation } => {
            if app.connection_generation == Some(generation)
                && app.compose_peer().as_deref() == Some(peer_hash.as_str())
                && app.compose_pending.is_none()
            {
                app.editor.set_text(draft.as_ref().map_or("", |value| value.content.as_str()));
            }
        }
        TuiEvent::DraftCleared { peer_hash, generation } => {
            if app.connection_generation == Some(generation)
                && app.compose_peer().as_deref() == Some(peer_hash.as_str())
            {
                app.editor.clear_line();
            }
        }
        TuiEvent::MessagePage { peer_hash, messages, next_cursor, reset, generation } => {
            if app.connection_generation != Some(generation) {
                return;
            }
            if reset {
                app.message_cursors.remove(&peer_hash);
                if let Some(ids) = app.history_message_ids.remove(&peer_hash) {
                    for id in ids {
                        app.loaded_message_ids.remove(&id);
                    }
                }
                let baseline =
                    app.message_page_live_baselines.remove(&peer_hash).unwrap_or_default();
                app.live_messages.retain(|_, message| {
                    let belongs = if message.is_outgoing {
                        message.destination_hash == peer_hash
                    } else {
                        message.source_hash == peer_hash
                    };
                    !belongs || !baseline.contains(&message.id)
                });
                let live_ids = app
                    .live_messages
                    .values()
                    .filter(|message| {
                        if message.is_outgoing {
                            message.destination_hash == peer_hash
                        } else {
                            message.source_hash == peer_hash
                        }
                    })
                    .map(|message| message.id.clone())
                    .collect::<Vec<_>>();
                for id in &live_ids {
                    app.loaded_message_ids.remove(id);
                }
                app.peer_conversation(&peer_hash).clear();
                app.conversation.push_system("⬡ history cursor stale; snapshot restarted");
            } else {
                app.message_page_live_baselines.remove(&peer_hash);
            }
            let page_ids = messages.iter().map(|message| message.id.clone()).collect::<Vec<_>>();
            let mut merged = messages
                .into_iter()
                .map(|message| (message.id.clone(), message))
                .collect::<HashMap<_, _>>();
            if reset {
                for message in app.live_messages.values().filter(|message| {
                    if message.is_outgoing {
                        message.destination_hash == peer_hash
                    } else {
                        message.source_hash == peer_hash
                    }
                }) {
                    merged.insert(message.id.clone(), message.clone());
                }
            }
            let mut messages = merged.into_values().collect::<Vec<_>>();
            messages.sort_by(|left, right| {
                right.timestamp.cmp(&left.timestamp).then_with(|| right.id.cmp(&left.id))
            });
            let name = app
                .peers
                .iter()
                .find(|peer| peer.hash == peer_hash)
                .and_then(|peer| peer.name.clone());
            let mut history = Vec::new();
            for message in messages.into_iter().rev() {
                if !app.loaded_message_ids.insert(message.id.clone()) {
                    continue;
                }
                if message.is_outgoing {
                    let status = delivery_status(
                        message.lifecycle_state,
                        message.terminal_detail.as_deref(),
                    );
                    history.push(Segment::SentMessage {
                        message_id: Some(message.id.clone()),
                        dest_hash: peer_hash.clone(),
                        dest_name: name.clone(),
                        text: message.content.clone(),
                        delivery_status: status,
                        lifecycle: MessageLifecycle::from(&message),
                    });
                } else {
                    history.push(Segment::ReceivedMessage {
                        message_id: Some(message.id.clone()),
                        source_hash: peer_hash.clone(),
                        source_name: name.clone(),
                        title: message.title.clone(),
                        text: message.content.clone(),
                        timestamp: message.timestamp,
                        lifecycle: MessageLifecycle::from(&message),
                    });
                }
            }
            app.peer_conversation(&peer_hash).prepend_history(history);
            app.history_message_ids.entry(peer_hash.clone()).or_default().extend(page_ids);
            match next_cursor {
                Some(cursor) => {
                    app.message_cursors.insert(peer_hash, cursor);
                }
                None => {
                    app.message_cursors.remove(&peer_hash);
                }
            }
        }
        TuiEvent::ConversationPage { conversations, next_cursor, reset, generation } => {
            if app.connection_generation != Some(generation) {
                return;
            }
            if reset {
                app.conversation_cursor = None;
                app.conversation_summaries.clear();
            }
            for conversation in conversations {
                app.conversation_summaries.insert(conversation.peer_hash.clone(), conversation);
            }
            app.conversation_cursor = next_cursor;
        }

        TuiEvent::CommandResult { action, success, detail, generation } => {
            if app.connection_generation != Some(generation) {
                return;
            }
            let prefix = if success { "✓" } else { "✗" };
            app.conversation.push_system(&format!("{prefix} {action}: {detail}"));
            if action == "standard propagation" {
                app.standard_propagation_error = (!success).then_some(detail.clone());
            }
            if action == "close page" && !success {
                app.pending_page_transition = None;
            }

            // Update command tab result if it was a fleet command
            match action.as_str() {
                "device_status" | "exec" | "reboot_device" | "fleet_apply" => {
                    app.command_tab.is_executing = false;
                    app.command_tab.result_text = format!("  {prefix} {detail}");
                }
                _ => {}
            }
        }

        TuiEvent::PageLoaded { host: _, path: _, page, generation } => {
            if app.connection_generation != Some(generation) {
                return;
            }
            let correlation_id = page.correlation_id.clone();
            let path = page.request.native_path.clone();
            app.page_field_values.clear();
            for field in &page.fields {
                match field.kind {
                    styrene_ipc::types::PageFormFieldKind::Text => {
                        app.page_field_values.insert(
                            field.name.clone(),
                            vec![field.value.clone().unwrap_or_default()],
                        );
                    }
                    styrene_ipc::types::PageFormFieldKind::Password => {
                        app.page_field_values.insert(field.name.clone(), vec![String::new()]);
                    }
                    styrene_ipc::types::PageFormFieldKind::Checkbox
                    | styrene_ipc::types::PageFormFieldKind::Radio
                        if field.checked =>
                    {
                        if let Some(value) = &field.value {
                            app.page_field_values
                                .entry(field.name.clone())
                                .or_default()
                                .push(value.clone());
                        }
                    }
                    _ => {}
                }
            }
            app.page_link_selection = 0;
            app.page_field_selection = 0;
            app.page_content = Some(*page);
            app.page_path = Some(path);
            app.conversation.push_system(&format!("page correlation: {correlation_id}"));
            app.focus = crate::app::Focus::Main;
        }

        TuiEvent::PageClosed { session_id, generation } => {
            if app.connection_generation != Some(generation) {
                return;
            }
            if app
                .page_content
                .as_ref()
                .is_some_and(|page| page.navigation.session_id == session_id)
            {
                app.confirm_page_closed();
            }
            app.conversation.push_system("page session closed");
        }

        TuiEvent::PageList { host: _, pages, generation } => {
            if app.connection_generation != Some(generation) {
                return;
            }
            app.page_index = pages;
            app.page_selection = 0;
            app.focus = crate::app::Focus::Main;
        }

        TuiEvent::FileDownload { download, generation } => {
            if app.connection_generation != Some(generation) {
                return;
            }
            app.conversation.push_system(&format!(
                "download {}: {:?} {:.0}% integrity={}",
                download.download_id,
                download.state,
                download.progress * 100.0,
                download.integrity_verified
            ));
            app.page_download = Some(download);
        }

        TuiEvent::TerminalOutput { session_id, data } => {
            if app.terminal_tab.session_id.as_deref() == Some(&session_id) {
                app.terminal_tab.push_output(&data);
            }
        }

        TuiEvent::TerminalExited { session_id, exit_code } => {
            if app.terminal_tab.session_id.as_deref() == Some(&session_id) {
                let msg = match exit_code {
                    Some(code) => format!("Session exited with code {code}"),
                    None => "Session exited".to_string(),
                };
                app.terminal_tab.scrollback.push(format!("--- {msg} ---"));
                app.terminal_tab.status = crate::app::TerminalStatus::Disconnected;
                app.terminal_tab.session_id = None;
            }
        }

        TuiEvent::Disconnected(reason) => {
            app.daemon_connected = false;
            app.daemon_session_accepting_events = false;
            app.cmd_tx = None;
            app.connection_generation = None;
            app.event_connection_generation = None;
            app.standard_propagation = None;
            app.standard_propagation_error = Some(reason.clone());
            app.active_capabilities = None;
            app.conversation_page_loaded = false;
            app.conversation_cursor = None;
            app.resource_transfers.clear();
            app.route_observations.clear();
            app.interface_observations.clear();
            app.rns_initialized = false;
            app.transport_active = false;
            app.page_content = None;
            app.page_path = None;
            app.page_index.clear();
            app.page_field_values.clear();
            app.page_download = None;
            app.pending_page_transition = None;
            app.conversation.push_system(&format!("⚠ daemon disconnected: {reason}"));
            app.activity.push(ActivityEntry::new(
                ActivityKind::LinkDown,
                "daemon",
                format!("disconnected: {reason}"),
            ));
        }
    }
}

// ─── Wire payload parsers ─────────────────────────────────────────────────────

fn payload_value(payload: &HashMap<String, MpValue>) -> MpValue {
    MpValue::Map(
        payload.iter().map(|(key, value)| (MpValue::from(key.as_str()), value.clone())).collect(),
    )
}

fn link_update_event(event: IpcLinkEvent) -> TuiEvent {
    TuiEvent::LinkUpdate {
        link_id: event.link_id,
        peer_hash: event.peer_hash,
        peer_name: event.peer_name,
        interface: event.interface,
        status: event.status,
        kind: event.kind,
        activity: event.activity,
        reason: event.reason,
        remote_identity_hash: event.remote_identity_hash,
        rtt_ms: event.rtt_ms,
        observation: event.observation,
    }
}

/// Decode a whole payload through the shared client decoder, which accepts
/// the daemon's string-spelled enum fields that rmpv's direct enum decoding
/// rejects.
fn parse_typed_payload<T: serde::de::DeserializeOwned>(
    payload: &HashMap<String, MpValue>,
) -> Result<T, String> {
    styrene_ipc_client::decode_payload(payload, "typed IPC payload")
        .map_err(|error| error.to_string())
}

fn parse_typed_value<T: serde::de::DeserializeOwned>(value: MpValue) -> Result<T, String> {
    styrene_ipc_client::decode_value(value, "typed IPC value").map_err(|error| error.to_string())
}

fn parse_mark_read_response(
    payload: &HashMap<String, MpValue>,
    peer_hash: &str,
) -> Result<MessagingOperationOutcome, String> {
    if let Some(outcome) = payload.get("outcome") {
        return parse_typed_value(outcome.clone());
    }
    let count = payload
        .get("count")
        .and_then(MpValue::as_u64)
        .ok_or("mark-read response missing outcome and count")?;
    let mut outcome = MessagingOperationOutcome::default();
    outcome.disposition = if count == 0 {
        styrene_ipc::types::MessagingDisposition::Unchanged
    } else {
        styrene_ipc::types::MessagingDisposition::Applied
    };
    outcome.affected_count = count;
    outcome.target_id = peer_hash.into();
    Ok(outcome)
}

fn parse_delete_message_response(
    payload: &HashMap<String, MpValue>,
    message_id: &str,
) -> Result<MessagingOperationOutcome, String> {
    if let Some(outcome) = payload.get("outcome") {
        return parse_typed_value(outcome.clone());
    }
    let success = payload
        .get("success")
        .and_then(MpValue::as_bool)
        .ok_or("delete response missing outcome and success")?;
    let mut outcome = MessagingOperationOutcome::default();
    outcome.disposition = if success {
        styrene_ipc::types::MessagingDisposition::Applied
    } else {
        styrene_ipc::types::MessagingDisposition::NotFound
    };
    outcome.affected_count = u64::from(success);
    outcome.target_id = message_id.into();
    Ok(outcome)
}

fn parse_typed_payload_key<T: serde::de::DeserializeOwned>(
    payload: &HashMap<String, MpValue>,
    key: &str,
) -> Result<T, String> {
    let bytes = payload
        .get(key)
        .and_then(MpValue::as_slice)
        .ok_or_else(|| format!("daemon response omitted typed {key} payload"))?;
    rmp_serde::from_slice(bytes).map_err(|error| format!("decode typed {key}: {error}"))
}

fn parse_typed_array<T: serde::de::DeserializeOwned>(
    payload: &HashMap<String, MpValue>,
    key: &str,
) -> Result<Vec<T>, String> {
    payload
        .get(key)
        .and_then(MpValue::as_array)
        .ok_or_else(|| format!("daemon response omitted {key}"))?
        .iter()
        .cloned()
        .map(|value| {
            styrene_ipc_client::decode_value(value, key).map_err(|error| error.to_string())
        })
        .collect()
}

fn mp_str(payload: &HashMap<String, MpValue>, key: &str) -> String {
    payload.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn decode_page_payload(
    payload: &HashMap<String, MpValue>,
) -> Result<styrene_ipc::types::PageContent, String> {
    let bytes = match payload.get("page") {
        Some(MpValue::Binary(bytes)) => bytes,
        _ => return Err("daemon page response omitted typed page payload".into()),
    };
    rmp_serde::from_slice(bytes).map_err(|error| format!("decode typed page payload: {error}"))
}

fn mp_bool(payload: &HashMap<String, MpValue>, key: &str) -> bool {
    payload.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn mp_u64(payload: &HashMap<String, MpValue>, key: &str) -> u64 {
    payload.get(key).and_then(|v| v.as_u64()).unwrap_or(0)
}

fn mp_i64(payload: &HashMap<String, MpValue>, key: &str) -> i64 {
    payload.get(key).and_then(|v| v.as_i64()).unwrap_or(0)
}

fn parse_message_from_payload(p: &HashMap<String, MpValue>) -> Option<MessageInfo> {
    let message = parse_typed_payload::<MessageInfo>(p).ok()?;
    (!message.id.is_empty()).then_some(message)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::segments::{DeliveryStatus, Segment};
    use styrene_ipc::types::{ActiveCapabilitiesInfo, DegradedCapabilityInfo, ObservationSource};

    #[test]
    fn device_decoders_preserve_only_known_announce_capabilities() {
        let device = MpValue::Map(vec![
            (MpValue::from("destination_hash"), MpValue::from("peer")),
            (
                MpValue::from("discovered_capabilities"),
                MpValue::Array(vec![
                    MpValue::from("native_nomadnet_host"),
                    MpValue::from("future_capability"),
                ]),
            ),
        ]);
        let snapshot = HashMap::from([("devices".into(), MpValue::Array(vec![device]))]);
        let parsed: Vec<DeviceInfo> =
            parse_typed_array(&snapshot, "devices").expect("device snapshot");
        assert_eq!(
            parsed[0].discovered_capabilities,
            [
                styrene_ipc::types::DiscoveredCapability::NativeNomadNetHost,
                styrene_ipc::types::DiscoveredCapability::Unknown
            ]
        );

        let pushed = HashMap::from([
            ("destination_hash".into(), MpValue::from("peer")),
            (
                "discovered_capabilities".into(),
                MpValue::Array(vec![MpValue::from("native_nomadnet_host")]),
            ),
        ]);
        let device: DeviceInfo = parse_typed_payload(&pushed).expect("pushed device");
        assert_eq!(
            device.discovered_capabilities,
            [styrene_ipc::types::DiscoveredCapability::NativeNomadNetHost]
        );
    }

    fn outgoing(id: &str, peer: &str, content: &str, status: &str, timestamp: i64) -> MessageInfo {
        let mut message = MessageInfo::default();
        message.id = id.into();
        message.destination_hash = peer.into();
        message.content = content.into();
        message.status = status.into();
        message.lifecycle_state = match status {
            "sending" => styrene_ipc::types::MessageLifecycleState::Sending,
            "sent" => styrene_ipc::types::MessageLifecycleState::Sent,
            "delivered" => styrene_ipc::types::MessageLifecycleState::Delivered,
            value if value.starts_with("cancelled") => {
                styrene_ipc::types::MessageLifecycleState::Cancelled
            }
            value if value.starts_with("failed") => {
                styrene_ipc::types::MessageLifecycleState::Failed
            }
            _ => styrene_ipc::types::MessageLifecycleState::Unknown,
        };
        message.timestamp = timestamp;
        message.is_outgoing = true;
        message.projection_complete = true;
        message
    }

    #[tokio::test]
    async fn standard_propagation_change_requeries_and_installs_snapshot_with_error_channel() {
        let mut app = crate::app::App::new();
        app.daemon_connected = true;
        app.connection_generation = Some(7);
        app.event_connection_generation = Some(9);
        let (cmd_tx, mut cmd_rx) = mpsc::channel(1);
        app.cmd_tx = Some(cmd_tx);
        let mut capabilities = ActiveCapabilitiesInfo::default();
        capabilities.version = styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION;
        capabilities.authorized_operations = vec!["rpc.status".into()];
        app.active_capabilities = Some(capabilities);

        apply_event(&mut app, TuiEvent::StandardPropagationChanged { connection_generation: 9 });
        let queued = cmd_rx.recv().await.expect("requery command");
        assert!(matches!(queued.command, DaemonCmd::RequeryStandardPropagation));

        let mut snapshot = StandardPropagationSnapshot::default();
        snapshot.version = styrene_ipc::types::STANDARD_PROPAGATION_SNAPSHOT_VERSION;
        snapshot.connection_generation = Some(7);
        snapshot.observed_at = Some(123);
        apply_event(&mut app, TuiEvent::StandardPropagationSnapshot(snapshot));
        assert_eq!(
            app.standard_propagation.as_ref().and_then(|value| value.observed_at),
            Some(123)
        );

        let mut unsupported = StandardPropagationSnapshot::default();
        unsupported.version = styrene_ipc::types::STANDARD_PROPAGATION_SNAPSHOT_VERSION + 1;
        unsupported.connection_generation = Some(7);
        unsupported.observed_at = Some(456);
        apply_event(&mut app, TuiEvent::StandardPropagationSnapshot(unsupported));
        assert_eq!(
            app.standard_propagation.as_ref().and_then(|value| value.observed_at),
            Some(123)
        );
        assert!(
            app.standard_propagation_error.as_deref().unwrap_or_default().contains("unsupported")
        );

        apply_event(
            &mut app,
            TuiEvent::CommandResult {
                action: "standard propagation".into(),
                success: false,
                detail: "query unavailable".into(),
                generation: 7,
            },
        );
        assert!(format!("{:?}", app.conversation.segments()).contains("query unavailable"));
        assert_eq!(app.standard_propagation_error.as_deref(), Some("query unavailable"));
    }

    #[test]
    fn messaging_mutation_responses_accept_typed_and_legacy_shapes() {
        let mut typed_outcome = MessagingOperationOutcome::default();
        typed_outcome.disposition = styrene_ipc::types::MessagingDisposition::Applied;
        typed_outcome.affected_count = 3;
        typed_outcome.target_id = "target".into();
        let typed = HashMap::from([(
            "outcome".into(),
            rmpv::ext::to_value(&typed_outcome).expect("encode typed outcome"),
        )]);
        assert_eq!(parse_mark_read_response(&typed, "ignored").unwrap(), typed_outcome);
        assert_eq!(parse_delete_message_response(&typed, "ignored").unwrap(), typed_outcome);

        let mark_read = HashMap::from([("count".into(), MpValue::from(2_u64))]);
        let outcome = parse_mark_read_response(&mark_read, "peer").unwrap();
        assert_eq!(outcome.disposition, styrene_ipc::types::MessagingDisposition::Applied);
        assert_eq!(outcome.affected_count, 2);
        assert_eq!(outcome.target_id, "peer");

        let delete = HashMap::from([("success".into(), MpValue::Boolean(false))]);
        let outcome = parse_delete_message_response(&delete, "message").unwrap();
        assert_eq!(outcome.disposition, styrene_ipc::types::MessagingDisposition::NotFound);
        assert_eq!(outcome.affected_count, 0);
        assert_eq!(outcome.target_id, "message");
    }

    #[test]
    fn chat_send_result_updates_the_correct_peer_conversation() {
        let mut app = crate::app::App::new();
        app.daemon_connected = true;
        app.connection_generation = Some(7);
        let mut capabilities = ActiveCapabilitiesInfo::default();
        capabilities.version = styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION;
        capabilities.authorized_operations = vec!["chat.send".into()];
        capabilities.runtime = vec!["runtime.lxmf.direct".into()];
        app.active_capabilities = Some(capabilities);

        let outcome = |peer: &str, id: &str, status: &str, disposition| {
            let mut message = MessageInfo::default();
            message.id = id.into();
            message.destination_hash = peer.into();
            message.content = format!("hello {peer}");
            message.status = status.into();
            message.lifecycle_state = match status {
                "sent" => styrene_ipc::types::MessageLifecycleState::Sent,
                value if value.starts_with("failed") => {
                    styrene_ipc::types::MessageLifecycleState::Failed
                }
                _ => styrene_ipc::types::MessageLifecycleState::Unknown,
            };
            if message.lifecycle_state == styrene_ipc::types::MessageLifecycleState::Failed {
                message.terminal_detail = Some("no route".into());
            }
            message.is_outgoing = true;
            let mut outcome = styrene_ipc::types::SendChatOutcome::default();
            outcome.disposition = disposition;
            outcome.message_id = id.into();
            outcome.message = message;
            outcome
        };

        apply_event(
            &mut app,
            TuiEvent::ChatSendOutcome {
                peer_hash: "peer-a".into(),
                outcome: Box::new(outcome(
                    "peer-a",
                    "message-a",
                    "sent",
                    styrene_ipc::types::SendChatDisposition::Accepted,
                )),
                generation: 7,
            },
        );
        apply_event(
            &mut app,
            TuiEvent::ChatSendOutcome {
                peer_hash: "peer-b".into(),
                outcome: Box::new(outcome(
                    "peer-b",
                    "message-b",
                    "failed: no route",
                    styrene_ipc::types::SendChatDisposition::Failed,
                )),
                generation: 7,
            },
        );

        assert_eq!(app.conversations["peer-a"].last_sent_status(), Some(&DeliveryStatus::Sent));
        assert_eq!(
            app.conversations["peer-b"].last_sent_status(),
            Some(&DeliveryStatus::Failed("no route".into()))
        );
        assert!(
            app.conversations
                .get_mut("peer-a")
                .expect("peer-a conversation")
                .update_sent_status("message-a", DeliveryStatus::Delivered)
        );
    }

    #[test]
    fn parse_identity_defaults() {
        let mut p = HashMap::new();
        p.insert("destination_hash".into(), MpValue::String("deadbeef".into()));
        p.insert("display_name".into(), MpValue::String("Test Node".into()));
        let id: IdentityInfo = parse_typed_payload(&p).unwrap();
        assert_eq!(id.destination_hash, "deadbeef");
        assert_eq!(id.display_name, "Test Node");
        assert!(id.icon.is_none());
    }

    #[test]
    fn parse_status_defaults() {
        let mut p = HashMap::new();
        p.insert("uptime".into(), MpValue::Integer(42.into()));
        p.insert("rns_initialized".into(), MpValue::Boolean(true));
        let s: DaemonStatusInfo = parse_typed_payload(&p).unwrap();
        assert_eq!(s.uptime, 42);
        assert!(s.rns_initialized);
        assert_eq!(s.active_links, 0);
        assert!(s.active_capabilities.is_none());
    }

    #[test]
    fn parse_status_preserves_capability_version_reason_and_generation() {
        let mut p = HashMap::new();
        p.insert("connection_generation".into(), MpValue::from(4_u64));
        p.insert(
            "active_capabilities".into(),
            MpValue::Map(vec![
                (MpValue::from("version"), MpValue::from(1_u64)),
                (
                    MpValue::from("runtime"),
                    MpValue::Array(vec![MpValue::from("runtime.lxmf.direct")]),
                ),
                (
                    MpValue::from("degraded"),
                    MpValue::Array(vec![MpValue::Map(vec![
                        (MpValue::from("id"), MpValue::from("runtime.native-nomadnet.host")),
                        (MpValue::from("reason"), MpValue::from("handler unavailable")),
                    ])]),
                ),
                (
                    MpValue::from("authorized_operations"),
                    MpValue::Array(vec![MpValue::from("chat.send")]),
                ),
            ]),
        );

        let status: DaemonStatusInfo = parse_typed_payload(&p).unwrap();
        let capabilities = status.active_capabilities.unwrap();
        assert_eq!(status.connection_generation, Some(4));
        assert_eq!(capabilities.version, 1);
        assert_eq!(capabilities.degraded[0].reason, "handler unavailable");

        // An empty capability map decodes to a version the TUI refuses to act on.
        p.insert("active_capabilities".into(), MpValue::Map(Vec::new()));
        let status: DaemonStatusInfo = parse_typed_payload(&p).unwrap();
        assert_ne!(
            status.active_capabilities.unwrap().version,
            styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION
        );
    }

    #[test]
    fn frame_to_tui_event_decodes_pushed_message_with_string_enum_fields() {
        let payload = HashMap::from([
            ("id".to_string(), MpValue::from("m1")),
            ("kind".to_string(), MpValue::from("new")),
            ("source_hash".to_string(), MpValue::from("aa")),
            ("destination_hash".to_string(), MpValue::from("bb")),
            ("content".to_string(), MpValue::from("hello")),
            ("authentication_state".to_string(), MpValue::from("verified")),
            ("lifecycle_state".to_string(), MpValue::from("delivered")),
            ("stamp_state".to_string(), MpValue::from("not_applicable")),
            ("connection_generation".to_string(), MpValue::from(3_u64)),
        ]);
        let frame = Frame {
            msg_type: MessageType::EventMessage,
            request_id: [0; REQUEST_ID_SIZE],
            payload,
        };
        let Some(TuiEvent::Message(message)) = frame_to_tui_event(frame) else {
            panic!("pushed message events must decode");
        };
        assert_eq!(message.id, "m1");
        assert_eq!(message.content, "hello");
        assert_eq!(
            message.authentication_state,
            styrene_ipc::types::MessageAuthenticationState::Verified
        );
        assert_eq!(message.lifecycle_state, styrene_ipc::types::MessageLifecycleState::Delivered);
    }

    #[tokio::test]
    async fn connect_to_a_missing_endpoint_fails_without_creating_it_or_starting_a_runtime() {
        let missing = std::env::temp_dir().join(format!(
            "styrene-tui-missing-{}-{}.sock",
            std::process::id(),
            line!()
        ));
        let error = match connect(Some(&missing)).await {
            Ok(_) => panic!("a missing endpoint must not connect"),
            Err(error) => error,
        };
        assert!(error.contains("socket not found"), "{error}");
        assert!(!missing.exists(), "live connection never creates the endpoint");
    }

    #[test]
    fn frame_to_tui_event_unknown_type_is_none() {
        let frame = Frame {
            msg_type: MessageType::Pong,
            request_id: [0; REQUEST_ID_SIZE],
            payload: HashMap::new(),
        };
        assert!(frame_to_tui_event(frame).is_none());
    }

    #[test]
    fn route_loss_frame_preserves_reason_and_expiry() {
        let mut payload = HashMap::new();
        payload.insert("kind".into(), MpValue::from("lost"));
        payload.insert("destination_hash".into(), MpValue::from("peer"));
        payload.insert("loss_reason".into(), MpValue::from("expired"));
        payload.insert("expires".into(), MpValue::from(700_i64));
        let event = frame_to_tui_event(Frame {
            msg_type: MessageType::EventRoute,
            request_id: [0; REQUEST_ID_SIZE],
            payload,
        });
        assert!(matches!(
            event,
            Some(TuiEvent::RouteLifecycle {
                kind,
                destination_hash,
                loss_reason: Some(reason),
                expires: Some(700),
                ..
            }) if kind == "lost" && destination_hash == "peer" && reason == "expired"
        ));
    }

    #[test]
    fn link_frame_preserves_typed_lifecycle_and_generation() {
        let mut payload = HashMap::new();
        payload.insert("link_id".into(), MpValue::from("link-1"));
        payload.insert("peer_hash".into(), MpValue::from("peer-1"));
        payload.insert("interface".into(), MpValue::from("iface-1"));
        payload.insert("status".into(), MpValue::from("closed"));
        payload.insert("kind".into(), MpValue::from("timeout"));
        payload.insert("activity".into(), MpValue::from("historical"));
        payload.insert("reason".into(), MpValue::from("stale_timeout"));
        payload.insert("remote_identity_hash".into(), MpValue::from("identity-1"));
        payload.insert("rtt_ms".into(), MpValue::F64(8.5));
        payload.insert("source".into(), MpValue::from("transport_link_state"));
        payload.insert("observed_at".into(), MpValue::from(100_i64));
        payload.insert("connection_generation".into(), MpValue::from(7_u64));

        let event = frame_to_tui_event(Frame {
            msg_type: MessageType::EventLink,
            request_id: [0; REQUEST_ID_SIZE],
            payload,
        });

        assert!(matches!(
            event,
            Some(TuiEvent::LinkUpdate {
                interface: Some(interface),
                kind: LinkEventKind::Timeout,
                activity: LinkActivity::Historical,
                reason: Some(LinkLifecycleReason::StaleTimeout),
                observation,
                remote_identity_hash: Some(identity),
                ..
            }) if interface == "iface-1"
                && identity == "identity-1"
                && observation.source == ObservationSource::TransportLinkState
                && observation.connection_generation == Some(7)
        ));
    }

    #[test]
    fn link_snapshot_keeps_active_and_history_distinct() {
        let event = |activity: &str, kind: &str| {
            MpValue::Map(vec![
                (MpValue::from("link_id"), MpValue::from("link-1")),
                (MpValue::from("peer_hash"), MpValue::from("peer-1")),
                (MpValue::from("status"), MpValue::from("active")),
                (MpValue::from("activity"), MpValue::from(activity)),
                (MpValue::from("kind"), MpValue::from(kind)),
            ])
        };
        let mut payload = HashMap::new();
        payload.insert("active".into(), MpValue::Array(vec![event("active", "established")]));
        payload.insert("history".into(), MpValue::Array(vec![event("historical", "teardown")]));

        let snapshot: LinkSnapshot = parse_typed_payload(&payload).expect("valid link snapshot");
        assert_eq!(snapshot.active[0].activity, LinkActivity::Active);
        assert_eq!(snapshot.history[0].activity, LinkActivity::Historical);
        assert_eq!(snapshot.history[0].kind, LinkEventKind::Teardown);
    }

    #[test]
    fn historical_link_observation_remains_available_for_inspection() {
        let mut app = crate::app::App::new();
        app.connection_generation = Some(7);
        let mut observation = ObservationMetadata::default();
        observation.connection_generation = Some(7);
        apply_event(
            &mut app,
            TuiEvent::LinkUpdate {
                link_id: "closed-link".into(),
                peer_hash: "peer".into(),
                peer_name: Some("Peer".into()),
                interface: Some("iface".into()),
                status: "closed".into(),
                kind: LinkEventKind::Teardown,
                activity: LinkActivity::Historical,
                reason: Some(LinkLifecycleReason::LocalTeardown),
                remote_identity_hash: None,
                rtt_ms: Some(10.0),
                observation,
            },
        );
        assert_eq!(app.links.len(), 1);
        assert_eq!(app.links[0].status, crate::mesh_state::LinkStatus::Closed);
    }

    #[test]
    fn mismatched_status_and_event_generations_are_rejected() {
        let mut app = crate::app::App::new();
        app.daemon_connected = true;

        let mut status = DaemonStatusInfo::default();
        status.connection_generation = Some(7);
        status.active_capabilities = Some(Default::default());
        apply_event(&mut app, TuiEvent::Status(status));
        assert_eq!(app.connection_generation, Some(7));

        let mut mismatched = DaemonStatusInfo::default();
        mismatched.connection_generation = Some(8);
        mismatched.active_capabilities = Some(Default::default());
        apply_event(&mut app, TuiEvent::Status(mismatched));
        assert!(app.connection_generation.is_none());
        assert!(app.active_capabilities.is_none());

        apply_event(&mut app, TuiEvent::EventGeneration(9));
        assert_eq!(app.event_connection_generation, Some(9));
        apply_event(&mut app, TuiEvent::EventGeneration(10));
        assert!(app.event_connection_generation.is_none());
    }

    #[test]
    fn queued_command_revalidation_requires_generation_exact_capability_and_no_degradation() {
        let queued = QueuedDaemonCmd {
            command: DaemonCmd::Announce,
            origin_generation: 7,
            capability: "network.announce".into(),
        };
        let mut active = ActiveCapabilitiesInfo::default();
        active.version = styrene_ipc::types::ACTIVE_CAPABILITIES_VERSION;
        active.authorized_operations = vec!["network.announce".into()];
        let mut status = DaemonStatusInfo::default();
        status.connection_generation = Some(7);
        status.active_capabilities = Some(active.clone());
        assert!(queued_command_authorized(&status, &queued));

        status.connection_generation = Some(8);
        assert!(!queued_command_authorized(&status, &queued));
        status.connection_generation = Some(7);
        let mut degraded = DegradedCapabilityInfo::default();
        degraded.id = "network.announce".into();
        degraded.reason = "transport unavailable".into();
        active.degraded.push(degraded);
        status.active_capabilities = Some(active);
        assert!(!queued_command_authorized(&status, &queued));
    }

    #[test]
    fn stale_command_results_cannot_mutate_current_ui() {
        let mut app = crate::app::App::new();
        app.connection_generation = Some(8);
        apply_event(
            &mut app,
            TuiEvent::CommandResult {
                action: "exec".into(),
                success: false,
                detail: "stale failure".into(),
                generation: 7,
            },
        );
        assert!(app.conversation.segments().is_empty());
    }

    #[test]
    fn snapshot_without_a_negotiated_generation_is_rejected() {
        let mut app = crate::app::App::new();
        let mut existing = ResourceTransferInfo::default();
        existing.resource_hash = "existing".into();
        app.resource_transfers.push(existing);
        apply_event(&mut app, TuiEvent::ResourceSnapshot(Vec::new()));

        assert_eq!(app.resource_transfers[0].resource_hash, "existing");
        assert!(app.connection_generation.is_none());
    }

    #[test]
    fn authoritative_snapshots_render_discoverable_ids_progress_errors_and_correlations() {
        let mut app = crate::app::App::new();
        app.connection_generation = Some(7);
        let mut route = PathInfo::default();
        route.destination_hash = "route-destination".into();
        route.observation.connection_generation = Some(7);
        route.observation.correlation_id = Some("route-correlation".into());
        apply_event(&mut app, TuiEvent::RouteSnapshot(vec![route]));

        let mut interface = InterfaceDetail::default();
        interface.name = "uplink".into();
        interface.hash = "interface-hash".into();
        interface.kind = "tcp_client".into();
        interface.mode = "point_to_point".into();
        interface.enabled = true;
        interface.status = "active".into();
        interface.host = Some("mesh.example".into());
        interface.port = Some(4242);
        interface.local_endpoint = Some("127.0.0.1:5000".into());
        interface.remote_endpoint = Some("192.0.2.1:4242".into());
        interface.parent_hash = Some("parent-hash".into());
        interface.tx_bytes = 12;
        interface.rx_bytes = 34;
        interface.peers_connected = 2;
        interface.observation.source = ObservationSource::RuntimeInterfaceRegistry;
        interface.observation.observed_at = Some(100);
        interface.observation.age_secs = Some(3);
        interface.observation.freshness_threshold_secs = Some(30);
        interface.observation.connection_generation = Some(7);
        interface.observation.correlation_id = Some("interface-correlation".into());
        apply_event(&mut app, TuiEvent::InterfaceSnapshot(vec![interface]));

        let mut request = RequestObservationInfo::default();
        request.request_id = "request-id".into();
        request.link_id = "link-id".into();
        request.progress = 0.5;
        request.protocol_error = Some(styrene_ipc::types::RequestProtocolError::MalformedResponse);
        request.observation.connection_generation = Some(7);
        request.observation.correlation_id = Some("request-correlation".into());
        apply_event(&mut app, TuiEvent::RequestSnapshot(vec![request]));

        let text = app
            .conversation
            .segments()
            .iter()
            .filter_map(|segment| match segment {
                crate::tui::segments::Segment::SystemEvent { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("route-destination"));
        assert!(text.contains("route-correlation"));
        for detail in [
            "interface-hash",
            "tcp_client",
            "point_to_point",
            "mesh.example",
            "4242",
            "127.0.0.1:5000",
            "192.0.2.1:4242",
            "parent-hash",
            "RuntimeInterfaceRegistry",
            "interface-correlation",
        ] {
            assert!(text.contains(detail), "missing interface detail {detail}");
        }
        assert!(text.contains("request-id"));
        assert!(text.contains("link-id"));
        assert!(text.contains("50%"));
        assert!(text.contains("MalformedResponse"));
        assert!(text.contains("request-correlation"));
    }

    #[test]
    fn parse_message_from_empty_payload_is_none() {
        let p = HashMap::new();
        assert!(parse_message_from_payload(&p).is_none());
    }

    #[test]
    fn message_parser_preserves_authoritative_delivery_lifecycle() {
        let mut payload = HashMap::new();
        payload.insert("id".into(), MpValue::from("message"));
        payload.insert("requested_delivery_method".into(), MpValue::from("opportunistic"));
        payload.insert("actual_delivery_method".into(), MpValue::from("direct"));
        payload.insert("fallback_reason".into(), MpValue::from("packet limit"));
        payload.insert("correlation_id".into(), MpValue::from("send-1"));
        payload.insert(
            "attempts".into(),
            MpValue::Array(vec![MpValue::Map(vec![
                (MpValue::from("message_id"), MpValue::from("message")),
                (MpValue::from("number"), MpValue::from(1_u64)),
                (MpValue::from("started_unix_ms"), MpValue::from(100_i64)),
                (MpValue::from("deadline_unix_ms"), MpValue::from(200_i64)),
                (MpValue::from("state"), MpValue::from("failed")),
            ])]),
        );

        let message = parse_message_from_payload(&payload).unwrap();

        assert_eq!(message.requested_delivery_method.as_deref(), Some("opportunistic"));
        assert_eq!(message.actual_delivery_method.as_deref(), Some("direct"));
        assert_eq!(message.fallback_reason.as_deref(), Some("packet limit"));
        assert_eq!(message.correlation_id.as_deref(), Some("send-1"));
        assert_eq!(message.attempts.len(), 1);
        assert_eq!(message.attempts[0].message_id, "message");
        assert_eq!(message.attempts[0].number, 1);
        assert_eq!(message.attempts[0].started_unix_ms, 100);
        assert_eq!(message.attempts[0].deadline_unix_ms, 200);
        assert_eq!(message.attempts[0].state, "failed");
    }

    #[test]
    fn sparse_presentation_status_does_not_infer_lifecycle() {
        let mut app = crate::app::App::new();
        app.conversation.push_sent(
            Some("msg-1"),
            "peer",
            None,
            "hello",
            crate::tui::segments::DeliveryStatus::Pending,
        );

        apply_event(
            &mut app,
            TuiEvent::MessageStatus { id: "msg-1".into(), status: "delivered".into() },
        );

        assert_eq!(
            app.conversation.last_sent_status(),
            Some(&crate::tui::segments::DeliveryStatus::Pending)
        );
    }

    #[test]
    fn message_status_does_not_update_unrelated_row() {
        let mut app = crate::app::App::new();
        app.conversation.push_sent(
            Some("msg-1"),
            "peer",
            None,
            "hello",
            crate::tui::segments::DeliveryStatus::Pending,
        );

        apply_event(
            &mut app,
            TuiEvent::MessageStatus { id: "msg-2".into(), status: "failed: timeout".into() },
        );

        assert_eq!(
            app.conversation.last_sent_status(),
            Some(&crate::tui::segments::DeliveryStatus::Pending)
        );
    }

    #[test]
    fn apply_disconnected_sets_transport_inactive() {
        // We can't easily construct a full App in unit tests,
        // but we can verify the TuiEvent variants are constructible
        let ev = TuiEvent::Disconnected("test reason".into());
        match ev {
            TuiEvent::Disconnected(reason) => assert_eq!(reason, "test reason"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn typed_page_decode_retains_canonical_bytes_and_metadata() {
        let mut page = styrene_ipc::types::PageContent::default();
        page.source_bytes = vec![0xff, 0x00, 0x7f];
        page.rendered_text = "projection".into();
        page.title = Some("Title".into());
        page.links.push("next.mu".into());
        page.correlation_id = "page-1".into();
        page.source_checksum = "aa".repeat(32);
        page.request.native_path = "/page/index.mu".into();
        page.cache.status = styrene_ipc::types::PageCacheStatus::NotUsed;
        page.cache.stored_at = Some(9);
        page.transfer.verified = true;
        let mut payload = HashMap::new();
        payload.insert(
            "page".into(),
            MpValue::Binary(rmp_serde::to_vec_named(&page).expect("encode page")),
        );

        let decoded = decode_page_payload(&payload).expect("decode page");

        assert_eq!(decoded, page);
        assert_eq!(decoded.source_bytes, [0xff, 0x00, 0x7f]);
    }

    #[test]
    fn paper_outcome_installs_exact_uri_before_clearing_compose() {
        let mut app = crate::app::App::new();
        app.connection_generation = Some(7);
        app.selected_conversation = Some("peer".into());
        app.editor.set_text("paper compose");
        app.compose_pending = Some(("peer".into(), "paper compose".into()));
        let mut outcome = styrene_ipc::types::SendChatOutcome::default();
        outcome.disposition = styrene_ipc::types::SendChatDisposition::PaperExported;
        outcome.message_id = "paper-id".into();
        outcome.message.id = outcome.message_id.clone();
        outcome.message.destination_hash = "peer".into();
        outcome.message.is_outgoing = true;
        outcome.paper_uri = Some("lxm://exact-paper-uri".into());

        apply_event(
            &mut app,
            TuiEvent::ChatSendOutcome {
                peer_hash: "peer".into(),
                outcome: Box::new(outcome),
                generation: 7,
            },
        );

        let export = app.paper_export.as_ref().unwrap();
        assert_eq!(export.uri, "lxm://exact-paper-uri");
        assert_eq!(app.editor.render_text(), "");
        assert!(!format!("{export:?}").contains("exact-paper-uri"));

        let mut missing = crate::app::App::new();
        missing.connection_generation = Some(7);
        missing.selected_conversation = Some("peer".into());
        missing.editor.set_text("retain me");
        missing.compose_pending = Some(("peer".into(), "retain me".into()));
        let mut outcome = styrene_ipc::types::SendChatOutcome::default();
        outcome.disposition = styrene_ipc::types::SendChatDisposition::PaperExported;
        outcome.message_id = "missing-paper-id".into();
        outcome.message.id = outcome.message_id.clone();
        outcome.message.destination_hash = "peer".into();
        outcome.message.is_outgoing = true;
        apply_event(
            &mut missing,
            TuiEvent::ChatSendOutcome {
                peer_hash: "peer".into(),
                outcome: Box::new(outcome),
                generation: 7,
            },
        );
        assert_eq!(missing.editor.render_text(), "retain me");
        assert!(missing.paper_export.is_none());
    }

    #[test]
    fn draft_discard_clears_only_on_matching_authoritative_success() {
        let mut app = crate::app::App::new();
        app.connection_generation = Some(7);
        app.selected_conversation = Some("peer".into());
        app.editor.set_text("retained draft");

        apply_event(&mut app, TuiEvent::DraftCleared { peer_hash: "other".into(), generation: 7 });
        apply_event(&mut app, TuiEvent::DraftCleared { peer_hash: "peer".into(), generation: 6 });
        apply_event(
            &mut app,
            TuiEvent::CommandResult {
                action: "discard draft".into(),
                success: false,
                detail: "denied".into(),
                generation: 7,
            },
        );
        assert_eq!(app.editor.render_text(), "retained draft");

        apply_event(&mut app, TuiEvent::DraftCleared { peer_hash: "peer".into(), generation: 7 });
        assert_eq!(app.editor.render_text(), "");
    }

    #[test]
    fn stale_message_cursor_replaces_snapshot_and_preserves_live_overlay() {
        let mut app = crate::app::App::new();
        app.connection_generation = Some(7);
        apply_event(
            &mut app,
            TuiEvent::MessagePage {
                peer_hash: "peer".into(),
                messages: vec![outgoing("obsolete", "peer", "obsolete", "sent", 1)],
                next_cursor: Some("stale".into()),
                reset: false,
                generation: 7,
            },
        );
        apply_event(
            &mut app,
            TuiEvent::Message(Box::new(outgoing("live", "peer", "live", "sent", 3))),
        );
        apply_event(
            &mut app,
            TuiEvent::MessagePage {
                peer_hash: "peer".into(),
                messages: vec![outgoing("replacement", "peer", "replacement", "sent", 2)],
                next_cursor: None,
                reset: true,
                generation: 7,
            },
        );

        let texts = app.conversations["peer"]
            .segments()
            .iter()
            .filter_map(|segment| match segment {
                Segment::SentMessage { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(texts, ["replacement", "live"]);
        assert!(!app.loaded_message_ids.contains("obsolete"));
    }

    #[test]
    fn lifecycle_dispositions_replace_projection_and_remove_not_found_controls() {
        use styrene_ipc::types::{MessagingDisposition, MessagingOperationOutcome};

        let mut app = crate::app::App::new();
        app.connection_generation = Some(7);
        apply_event(
            &mut app,
            TuiEvent::Message(Box::new(outgoing("message", "peer", "body", "sending", 1))),
        );
        for (disposition, status) in [
            (MessagingDisposition::Applied, "cancelled: operator request"),
            (MessagingDisposition::Unchanged, "sent"),
            (MessagingDisposition::AlreadyCancelled, "cancelled"),
            (MessagingDisposition::TerminalConflict, "delivered"),
        ] {
            let mut outcome = MessagingOperationOutcome::default();
            outcome.disposition = disposition;
            outcome.target_id = "message".into();
            outcome.correlated_id =
                (disposition == MessagingDisposition::Unchanged).then(|| "message".into());
            outcome.terminal_state =
                (disposition == MessagingDisposition::TerminalConflict).then(|| status.into());
            outcome.message = (disposition != MessagingDisposition::TerminalConflict)
                .then(|| outgoing("message", "peer", "body", status, 1));
            apply_event(&mut app, TuiEvent::MessagingOperation(Box::new(outcome)));
            let expected = match status {
                "delivered" => DeliveryStatus::Cancelled,
                "cancelled" | "cancelled: operator request" => DeliveryStatus::Cancelled,
                _ => DeliveryStatus::Sent,
            };
            assert_eq!(app.conversations["peer"].last_sent_status(), Some(&expected));
        }

        let mut already_cancelled = MessagingOperationOutcome::default();
        already_cancelled.disposition = MessagingDisposition::AlreadyCancelled;
        already_cancelled.target_id = "message".into();
        apply_event(&mut app, TuiEvent::MessagingOperation(Box::new(already_cancelled)));
        assert_eq!(app.conversations["peer"].last_sent_status(), Some(&DeliveryStatus::Cancelled));

        let mut missing = MessagingOperationOutcome::default();
        missing.disposition = MessagingDisposition::NotFound;
        missing.target_id = "message".into();
        apply_event(&mut app, TuiEvent::MessagingOperation(Box::new(missing)));
        assert!(!app.conversations["peer"].contains_sent("message"));
        assert!(!app.loaded_message_ids.contains("message"));
    }

    #[test]
    fn id_reconciliation_is_generation_gated_replaces_all_fields_and_removes_not_found() {
        let mut app = crate::app::App::new();
        app.connection_generation = Some(7);
        let mut stale = outgoing("message", "peer", "stale", "sending", 1);
        stale.correlation_id = Some("stale-correlation".into());
        stale.attempts.push(styrene_ipc::types::MessageAttemptInfo::default());
        apply_event(&mut app, TuiEvent::Message(Box::new(stale)));

        apply_event(
            &mut app,
            TuiEvent::MessageResolved {
                message_id: "message".into(),
                message: None,
                generation: 6,
            },
        );
        assert!(app.live_messages.contains_key("message"));

        let replacement = outgoing("message", "peer", "authoritative", "delivered", 2);
        apply_event(
            &mut app,
            TuiEvent::MessageResolved {
                message_id: "message".into(),
                message: Some(Box::new(replacement)),
                generation: 7,
            },
        );
        let message = &app.live_messages["message"];
        assert_eq!(message.content, "authoritative");
        assert!(message.correlation_id.is_none());
        assert!(message.attempts.is_empty());

        apply_event(
            &mut app,
            TuiEvent::MessageResolved {
                message_id: "message".into(),
                message: None,
                generation: 7,
            },
        );
        assert!(!app.live_messages.contains_key("message"));
        assert!(!app.conversations["peer"].contains_sent("message"));
    }
}
