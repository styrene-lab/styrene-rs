//! Generic HDLC-framed stream interface loops.
//!
//! Provides `run_hdlc_rx_loop` and `run_hdlc_tx_loop` — the shared
//! read→HDLC-decode→deserialize and serialize→HDLC-encode→write pipelines
//! used by all byte-stream transports (TCP, Serial/KISS, future WebSocket).
//!
//! Both functions are generic over Tokio's `AsyncRead` / `AsyncWrite` traits,
//! so adding a new stream transport (e.g. `tokio-serial`) requires only
//! constructing the stream and calling these functions — no boilerplate loop.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::buffer::{InputBuffer, OutputBuffer};
use crate::hash::AddressHash;
use crate::packet::Packet;
use crate::serde::Serialize;
use crate::transport::iface::{
    InterfaceDropReason, InterfaceRxSender, InterfaceStats, InterfaceTxReceiver, RxMessage,
};

use super::hdlc::Hdlc;
use super::ifac::IfacConfig;

// Per-interface buffer sizes.
//
// A wire frame carries one packet of at most `MAX_LINK_MTU` bytes plus the IFAC
// prefix and token. HDLC escaping can double a frame and adds two flag bytes,
// so the encode buffer must hold the fully escaped worst case: a link-MTU
// packet whose ciphertext happens to contain many flag or escape bytes must
// still be transmitted. Frames that fail to fit were previously dropped without
// any protocol-visible signal. The frame accumulator grows dynamically but is
// capped to prevent unbounded growth on malformed streams.
const IFAC_MAX_OVERHEAD: usize = 2 + 64;
const WIRE_FRAME_BUF: usize = crate::packet::MAX_LINK_MTU + IFAC_MAX_OVERHEAD;
const HDLC_BUF: usize = 2 * WIRE_FRAME_BUF + 2;
const TCP_READ_BUF: usize = HDLC_BUF * 16;
const FRAME_BUF_LIMIT: usize = HDLC_BUF * 64;

pub(crate) struct RxAdmission {
    mtu: usize,
    stats: Arc<InterfaceStats>,
}

impl RxAdmission {
    pub(crate) fn new(mtu: usize, stats: Arc<InterfaceStats>) -> Self {
        Self { mtu, stats }
    }
}

/// Run the receive half of an HDLC-framed byte-stream interface.
///
/// Reads bytes from `reader`, accumulates them in a frame buffer, finds and
/// decodes HDLC frames, optionally strips and verifies IFAC authentication,
/// deserializes RNS packets, and forwards them on `rx_channel`. Exits when
/// `cancel` or `stop` is triggered, or when the reader returns 0 bytes.
///
/// Suitable for any transport whose read half implements `AsyncRead + Unpin + Send`.
pub(crate) async fn run_hdlc_rx_loop<R>(
    mut reader: R,
    rx_channel: InterfaceRxSender,
    iface_address: AddressHash,
    cancel: CancellationToken,
    stop: CancellationToken,
    admission: RxAdmission,
    ifac: Option<Arc<IfacConfig>>,
) where
    R: tokio::io::AsyncRead + Unpin + Send,
{
    let mut hdlc_rx_buffer = [0u8; HDLC_BUF];
    let mut frame_buffer: Vec<u8> = Vec::with_capacity(TCP_READ_BUF);
    let mut read_buffer = vec![0u8; TCP_READ_BUF];

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = stop.cancelled() => break,
            result = reader.read(&mut read_buffer) => {
                match result {
                    Ok(0) => {
                        log::warn!("stream_iface: connection closed on {}", iface_address);
                        stop.cancel();
                        break;
                    }
                    Ok(n) => {
                        frame_buffer.extend_from_slice(&read_buffer[..n]);

                        while let Some((start, end)) = Hdlc::find(&frame_buffer) {
                            let frame = &frame_buffer[start..=end];
                            let mut output = OutputBuffer::new(&mut hdlc_rx_buffer);

                            if Hdlc::decode(frame, &mut output).is_ok() {
                                let raw = output.as_slice();

                                if raw.is_empty() {
                                    frame_buffer.drain(..=end);
                                    continue;
                                }

                                // IFAC: strip and verify if the interface requires it,
                                // or drop packets that carry IFAC on an Open interface.
                                let inner: Option<Vec<u8>> = if let Some(ref cfg) = ifac {
                                    // IFAC-enabled interface: must have valid IFAC token.
                                    let inner = super::ifac::ifac_unwrap(raw, cfg);
                                    if inner.is_none() {
                                        admission.stats.record_drop(InterfaceDropReason::IfacFailure);
                                    }
                                    inner
                                } else if !raw.is_empty() && raw[0] & 0x80 != 0 {
                                    // Open interface: reject packets with IFAC flag set.
                                    log::debug!(
                                        "stream_iface: dropping IFAC packet on open interface {}",
                                        iface_address
                                    );
                                    admission.stats.record_drop(InterfaceDropReason::IfacFailure);
                                    None
                                } else {
                                    Some(raw.to_vec())
                                };

                                if let Some(inner_bytes) = inner {
                                    if let Ok(packet) = Packet::deserialize(
                                        &mut InputBuffer::new(&inner_bytes),
                                    ) {
                                        let _ = rx_channel
                                            .send(RxMessage::physical(iface_address, packet, admission.mtu))
                                            .await;
                                    } else {
                                        admission.stats.record_drop(InterfaceDropReason::MalformedFrame);
                                        log::warn!(
                                            "stream_iface: packet deserialize failed on {}",
                                            iface_address
                                        );
                                    }
                                }
                            } else {
                                admission.stats.record_drop(InterfaceDropReason::MalformedFrame);
                                log::warn!(
                                    "stream_iface: HDLC decode failed on {}",
                                    iface_address
                                );
                            }

                            frame_buffer.drain(..=end);
                        }

                        if frame_buffer.len() > FRAME_BUF_LIMIT {
                            admission.stats.record_drop(InterfaceDropReason::MalformedFrame);
                            log::warn!(
                                "stream_iface: frame buffer overflow on {}, clearing",
                                iface_address
                            );
                            frame_buffer.clear();
                        }
                    }
                    Err(e) => {
                        log::warn!("stream_iface: read error on {}: {}", iface_address, e);
                        stop.cancel();
                        break;
                    }
                }
            }
        }
    }
}

/// Run the transmit half of an HDLC-framed byte-stream interface.
///
/// Receives `TxMessage`s from `tx_channel`, serializes packets, optionally
/// wraps with IFAC authentication, HDLC-encodes, and writes to `writer`.
/// Exits when `cancel` or `stop` is triggered, or on write error.
///
/// Suitable for any transport whose write half implements `AsyncWrite + Unpin + Send`.
/// Write queued messages to one stream. `epoch` is the connection epoch this
/// stream was published under; a message accepted for another epoch is
/// discarded rather than replayed over a connection it was never bound to.
#[allow(clippy::too_many_arguments)]
pub async fn run_hdlc_tx_loop<W>(
    mut writer: W,
    tx_channel: Arc<Mutex<InterfaceTxReceiver>>,
    iface_address: AddressHash,
    cancel: CancellationToken,
    stop: CancellationToken,
    ifac: Option<Arc<IfacConfig>>,
    epoch: u64,
    stats: Arc<InterfaceStats>,
) where
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let mut hdlc_tx_buffer = [0u8; HDLC_BUF];
    let mut tx_buffer = [0u8; WIRE_FRAME_BUF];

    loop {
        if stop.is_cancelled() {
            break;
        }

        let mut tx_channel_guard = tx_channel.lock().await;

        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = stop.cancelled() => break,
            Some(message) = tx_channel_guard.recv() => {
                drop(tx_channel_guard);
                if message.epoch != epoch {
                    stats.record_tx_stale_epoch();
                    log::debug!(
                        "stream_iface: discarding egress on {} from epoch {} (stream epoch {})",
                        iface_address,
                        message.epoch,
                        epoch
                    );
                    continue;
                }
                let packet = message.packet;
                let mut output = OutputBuffer::new(&mut tx_buffer);

                if packet.serialize(&mut output).is_ok() {
                    // IFAC: wrap the serialized packet if this interface requires it.
                    let wire_bytes: Vec<u8> = if let Some(ref cfg) = ifac {
                        super::ifac::ifac_wrap(output.as_slice(), cfg)
                    } else {
                        output.as_slice().to_vec()
                    };

                    let mut hdlc_output = OutputBuffer::new(&mut hdlc_tx_buffer);

                    if Hdlc::encode(&wire_bytes, &mut hdlc_output).is_ok() {
                        if let Err(e) = writer.write_all(hdlc_output.as_slice()).await {
                            log::warn!(
                                "stream_iface: write_all failed on {}: {}",
                                iface_address,
                                e
                            );
                            stop.cancel();
                            break;
                        }
                        if let Err(e) = writer.flush().await {
                            log::warn!(
                                "stream_iface: flush failed on {}: {}",
                                iface_address,
                                e
                            );
                            stop.cancel();
                            break;
                        }
                    } else {
                        log::warn!(
                            "stream_iface: HDLC encode failed on {}", iface_address
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::PacketDataBuffer;
    use crate::transport::iface::{InterfaceChannel, QueuedTx, TxMessage, TxMessageType};
    use tokio::time::{Duration, timeout};

    fn frame(data: &[u8]) -> Vec<u8> {
        let mut storage = vec![0_u8; data.len() * 2 + 2];
        let mut output = OutputBuffer::new(&mut storage);
        Hdlc::encode(data, &mut output).expect("HDLC frame");
        output.as_slice().to_vec()
    }

    #[tokio::test]
    async fn empty_malformed_and_ifac_frames_do_not_stop_stream_ingress() {
        let (mut writer, reader) = tokio::io::duplex(4096);
        let (rx_send, mut rx_recv) = InterfaceChannel::make_rx_channel(4);
        let address = AddressHash::new([0x41; 16]);
        let cancel = CancellationToken::new();
        let stop = CancellationToken::new();
        let stats = Arc::new(InterfaceStats::new());
        let task = tokio::spawn(run_hdlc_rx_loop(
            reader,
            rx_send,
            address,
            cancel.clone(),
            stop,
            RxAdmission::new(500, stats.clone()),
            None,
        ));

        let valid = Packet {
            data: PacketDataBuffer::new_from_slice(b"valid after drops"),
            ..Default::default()
        }
        .to_bytes()
        .expect("valid packet");
        let mut input = frame(&[]);
        input.extend(frame(&[0x01]));
        input.extend(frame(&[0x80]));
        input.extend(frame(&valid));
        writer.write_all(&input).await.expect("stream input");

        let received = timeout(Duration::from_secs(1), rx_recv.recv())
            .await
            .expect("valid frame deadline")
            .expect("valid frame");
        assert_eq!(received.packet.data.as_slice(), b"valid after drops");
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.violations.malformed_frame, 1);
        assert_eq!(snapshot.violations.ifac_failure, 1);
        assert_eq!(snapshot.filters.valid_blackhole, 0);

        cancel.cancel();
        task.await.expect("stream worker must not panic");
    }

    /// A link-MTU packet whose payload is entirely flag bytes escapes to
    /// almost twice its size. The transmit path must still frame and write it;
    /// it used to fail HDLC encoding against a 2 KB buffer and drop the frame
    /// silently, which lost resource parts on high-MTU links.
    #[tokio::test]
    async fn worst_case_escaped_link_mtu_frame_is_transmitted_intact() {
        let (writer, mut reader) = tokio::io::duplex(16 * 1024);
        let (tx_send, tx_recv) = InterfaceChannel::make_tx_channel(4);
        let address = AddressHash::new([0x42; 16]);
        let cancel = CancellationToken::new();
        let stop = CancellationToken::new();
        let task = tokio::spawn(run_hdlc_tx_loop(
            writer,
            Arc::new(Mutex::new(tx_recv)),
            address,
            cancel.clone(),
            stop,
            None,
            0,
            Arc::new(InterfaceStats::new()),
        ));

        // The same shape as a resource part on a 2048-byte link: a link data
        // packet carrying a 2012-byte payload made entirely of flag bytes.
        let payload = vec![HDLC_FRAME_FLAG_FOR_TEST; 2012];
        let packet = Packet {
            header: crate::packet::Header {
                destination_type: crate::packet::DestinationType::Link,
                packet_type: crate::packet::PacketType::Data,
                ..Default::default()
            },
            destination: AddressHash::new([0x7e; 16]),
            context: crate::packet::PacketContext::Resource,
            data: PacketDataBuffer::new_from_slice(&payload),
            ..Default::default()
        };
        let serialized = packet.to_bytes().expect("serialize link-MTU packet");
        let expected = frame(&serialized);
        assert!(expected.len() > 2048, "test frame must exceed the old 2 KB encode buffer");

        tx_send
            .send(QueuedTx::untracked(TxMessage {
                tx_type: TxMessageType::Broadcast(None),
                packet,
            }))
            .await
            .expect("queue frame");

        let mut received = vec![0_u8; expected.len()];
        timeout(Duration::from_secs(2), reader.read_exact(&mut received))
            .await
            .expect("frame write deadline")
            .expect("frame bytes");
        assert_eq!(received, expected, "escaped frame was truncated or corrupted");

        cancel.cancel();
        task.await.expect("transmit worker must not panic");
    }

    const HDLC_FRAME_FLAG_FOR_TEST: u8 = 0x7e;
}
