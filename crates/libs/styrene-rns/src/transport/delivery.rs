use crate::destination::DestinationDesc;
use crate::packet::Packet;
use crate::transport::core_transport::{SendPacketOutcome, Transport};
use crate::transport::destination_ext::link::{Link, LinkEvent, LinkStatus};
use crate::transport::error::RnsError;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub enum LinkSendResult {
    Packet(Box<Packet>),
    Resource(crate::hash::Hash),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSendKind {
    Packet,
    Resource,
}

pub fn send_outcome_label(outcome: SendPacketOutcome) -> &'static str {
    match outcome {
        SendPacketOutcome::SentDirect => "sent direct",
        SendPacketOutcome::SentBroadcast => "sent broadcast",
        SendPacketOutcome::DroppedMissingDestinationIdentity => "missing destination identity",
        SendPacketOutcome::DroppedCiphertextTooLarge => "ciphertext too large",
        SendPacketOutcome::DroppedEncryptFailed => "encrypt failed",
        SendPacketOutcome::DroppedNoRoute => "no route",
    }
}

pub fn send_outcome_is_sent(outcome: SendPacketOutcome) -> bool {
    matches!(outcome, SendPacketOutcome::SentDirect | SendPacketOutcome::SentBroadcast)
}

pub fn send_outcome_status(method: &str, outcome: SendPacketOutcome) -> String {
    match outcome {
        SendPacketOutcome::SentDirect | SendPacketOutcome::SentBroadcast => {
            format!("sent: {method}")
        }
        SendPacketOutcome::DroppedMissingDestinationIdentity => {
            format!("failed: {method} missing destination identity")
        }
        SendPacketOutcome::DroppedCiphertextTooLarge => {
            format!("failed: {method} payload too large")
        }
        SendPacketOutcome::DroppedEncryptFailed => format!("failed: {method} encrypt failed"),
        SendPacketOutcome::DroppedNoRoute => format!("failed: {method} no route"),
    }
}

pub fn strip_destination_prefix<'a>(payload: &'a [u8], destination: &[u8; 16]) -> &'a [u8] {
    if payload.len() > 16 && payload[..16] == destination[..] { &payload[16..] } else { payload }
}

pub async fn send_via_link(
    transport: &Transport,
    destination: DestinationDesc,
    payload: &[u8],
    wait_timeout: Duration,
) -> io::Result<LinkSendResult> {
    let link = transport.link(destination).await;
    await_link_activation(transport, &link, wait_timeout).await?;
    let link_id = *link.lock().await.id();

    let packet = {
        let guard = link.lock().await;
        guard.data_packet(payload)
    };

    match packet {
        Ok(packet) => {
            let outcome = transport.send_link_packet(&link, packet).await;
            if !send_outcome_is_sent(outcome) {
                return Err(io::Error::other(format!(
                    "link packet not sent: {}",
                    send_outcome_label(outcome)
                )));
            }
            Ok(LinkSendResult::Packet(Box::new(packet)))
        }
        Err(RnsError::OutOfMemory | RnsError::InvalidArgument) => {
            let resource_hash = transport
                .send_resource(&link_id, payload.to_vec(), None)
                .await
                .map_err(|err| io::Error::other(format!("link resource not sent: {err:?}")))?;
            Ok(LinkSendResult::Resource(resource_hash))
        }
        Err(err) => Err(io::Error::other(format!("{err:?}"))),
    }
}

/// Send on an already-active link without opening or selecting another link.
pub async fn send_over_link(
    transport: &Transport,
    link_id: &crate::hash::AddressHash,
    payload: &[u8],
) -> io::Result<LinkSendResult> {
    let link = transport
        .find_out_link(link_id)
        .await
        .ok_or_else(|| io::Error::other("active outbound link not found"))?;
    let packet = {
        let guard = link.lock().await;
        if guard.status() != LinkStatus::Active {
            return Err(io::Error::other("outbound link is not active"));
        }
        guard.data_packet(payload)
    };
    match packet {
        Ok(packet) => {
            let outcome = transport.send_link_packet(&link, packet).await;
            if !send_outcome_is_sent(outcome) {
                return Err(io::Error::other(format!(
                    "link packet not sent: {}",
                    send_outcome_label(outcome)
                )));
            }
            Ok(LinkSendResult::Packet(Box::new(packet)))
        }
        Err(RnsError::OutOfMemory | RnsError::InvalidArgument) => transport
            .send_resource(link_id, payload.to_vec(), None)
            .await
            .map(LinkSendResult::Resource)
            .map_err(|error| io::Error::other(format!("link resource not sent: {error:?}"))),
        Err(error) => Err(io::Error::other(format!("{error:?}"))),
    }
}

/// Send via a link while preserving cancellation ownership until dispatch.
///
/// `before_dispatch` is the linearization point: cancellation that wins before
/// it returns prevents transmission; cancellation after it returns must treat
/// the selected transport representation as dispatched.
pub async fn send_via_link_cancellable<F>(
    transport: &Transport,
    destination: DestinationDesc,
    payload: &[u8],
    wait_timeout: Duration,
    cancellation: CancellationToken,
    before_dispatch: F,
) -> io::Result<LinkSendResult>
where
    F: FnOnce(LinkSendKind) -> io::Result<()>,
{
    use crate::transport::core_transport::LinkDispatch;

    let dispatch = transport
        .link_cancellable(destination, cancellation.clone())
        .await
        .ok_or_else(|| io::Error::new(io::ErrorKind::Interrupted, "link send cancelled"))?;
    let (link, created) = match dispatch {
        LinkDispatch::Created(link) => (link, true),
        LinkDispatch::Reused(link) => (link, false),
    };
    let link_id = *link.lock().await.id();

    let activation = await_link_activation(transport, &link, wait_timeout);
    tokio::select! {
        biased;
        () = cancellation.cancelled() => {
            if created {
                transport.close_link(&link_id).await;
            }
            return Err(io::Error::new(io::ErrorKind::Interrupted, "link send cancelled"));
        }
        result = activation => result?,
    }

    if cancellation.is_cancelled() {
        if created {
            transport.close_link(&link_id).await;
        }
        return Err(io::Error::new(io::ErrorKind::Interrupted, "link send cancelled"));
    }

    let packet = {
        let guard = link.lock().await;
        guard.data_packet(payload)
    };

    match packet {
        Ok(packet) => {
            before_dispatch(LinkSendKind::Packet)?;
            let outcome = transport.send_link_packet(&link, packet).await;
            if !send_outcome_is_sent(outcome) {
                return Err(io::Error::other(format!(
                    "link packet not sent: {}",
                    send_outcome_label(outcome)
                )));
            }
            Ok(LinkSendResult::Packet(Box::new(packet)))
        }
        Err(RnsError::OutOfMemory | RnsError::InvalidArgument) => {
            before_dispatch(LinkSendKind::Resource)?;
            let resource_hash = transport
                .send_resource(&link_id, payload.to_vec(), None)
                .await
                .map_err(|err| io::Error::other(format!("link resource not sent: {err:?}")))?;
            Ok(LinkSendResult::Resource(resource_hash))
        }
        Err(err) => Err(io::Error::other(format!("{err:?}"))),
    }
}

pub async fn await_link_activation(
    transport: &Transport,
    link: &Arc<tokio::sync::Mutex<Link>>,
    wait_timeout: Duration,
) -> io::Result<()> {
    let link_id = *link.lock().await.id();
    if link.lock().await.status() == LinkStatus::Active {
        return Ok(());
    }

    let mut events = transport.out_link_events();
    let deadline = tokio::time::Instant::now() + wait_timeout;
    loop {
        if link.lock().await.status() == LinkStatus::Active {
            return Ok(());
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "link activation timed out"));
        }

        let wait_slice = remaining.min(Duration::from_millis(250));
        match tokio::time::timeout(wait_slice, events.recv()).await {
            Ok(Ok(event)) => {
                if event.id == link_id && matches!(event.event, LinkEvent::Activated) {
                    return Ok(());
                }
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "link event channel closed",
                ));
            }
            Err(_) => continue,
        }
    }
}
