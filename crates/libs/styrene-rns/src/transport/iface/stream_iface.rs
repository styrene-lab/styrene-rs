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
use crate::transport::iface::{InterfaceRxSender, InterfaceTxReceiver, RxMessage};

use super::hdlc::Hdlc;
use super::ifac::IfacConfig;

// Per-interface buffer sizes. 2 KB per buffer; frame accumulator grows
// dynamically but is capped to prevent unbounded growth on malformed streams.
const HDLC_BUF: usize = 2048;
const TCP_READ_BUF: usize = HDLC_BUF * 16;
const FRAME_BUF_LIMIT: usize = HDLC_BUF * 64;

/// Run the receive half of an HDLC-framed byte-stream interface.
///
/// Reads bytes from `reader`, accumulates them in a frame buffer, finds and
/// decodes HDLC frames, optionally strips and verifies IFAC authentication,
/// deserializes RNS packets, and forwards them on `rx_channel`. Exits when
/// `cancel` or `stop` is triggered, or when the reader returns 0 bytes.
///
/// Suitable for any transport whose read half implements `AsyncRead + Unpin + Send`.
pub async fn run_hdlc_rx_loop<R>(
    mut reader: R,
    rx_channel: InterfaceRxSender,
    iface_address: AddressHash,
    cancel: CancellationToken,
    stop: CancellationToken,
    ifac: Option<&IfacConfig>,
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

                                // IFAC: strip and verify if the interface requires it,
                                // or drop packets that carry IFAC on an Open interface.
                                let inner: Option<Vec<u8>> = if let Some(cfg) = ifac {
                                    // IFAC-enabled interface: must have valid IFAC token.
                                    super::ifac::ifac_unwrap(raw, cfg)
                                } else if !raw.is_empty() && raw[0] & 0x80 != 0 {
                                    // Open interface: reject packets with IFAC flag set.
                                    log::debug!(
                                        "stream_iface: dropping IFAC packet on open interface {}",
                                        iface_address
                                    );
                                    None
                                } else {
                                    Some(raw.to_vec())
                                };

                                if let Some(inner_bytes) = inner {
                                    if let Ok(packet) = Packet::deserialize(
                                        &mut InputBuffer::new(&inner_bytes),
                                    ) {
                                        let _ = rx_channel
                                            .send(RxMessage { address: iface_address, packet })
                                            .await;
                                    } else {
                                        log::warn!(
                                            "stream_iface: packet deserialize failed on {}",
                                            iface_address
                                        );
                                    }
                                }
                            } else {
                                log::warn!(
                                    "stream_iface: HDLC decode failed on {}",
                                    iface_address
                                );
                            }

                            frame_buffer.drain(..=end);
                        }

                        if frame_buffer.len() > FRAME_BUF_LIMIT {
                            log::warn!(
                                "stream_iface: frame buffer overflow on {}, clearing",
                                iface_address
                            );
                            frame_buffer.clear();
                        }
                    }
                    Err(e) => {
                        log::warn!("stream_iface: read error on {}: {}", iface_address, e);
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
pub async fn run_hdlc_tx_loop<W>(
    mut writer: W,
    tx_channel: Arc<Mutex<InterfaceTxReceiver>>,
    iface_address: AddressHash,
    cancel: CancellationToken,
    stop: CancellationToken,
    ifac: Option<&IfacConfig>,
) where
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let mut hdlc_tx_buffer = [0u8; HDLC_BUF];
    let mut tx_buffer = [0u8; HDLC_BUF];

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
                let packet = message.packet;
                let mut output = OutputBuffer::new(&mut tx_buffer);

                if packet.serialize(&mut output).is_ok() {
                    // IFAC: wrap the serialized packet if this interface requires it.
                    let wire_bytes: Vec<u8> = if let Some(cfg) = ifac {
                        super::ifac::ifac_wrap(output.as_slice(), cfg)
                    } else {
                        output.as_slice().to_vec()
                    };

                    let mut hdlc_output = OutputBuffer::new(&mut hdlc_tx_buffer);

                    if Hdlc::encode(&wire_bytes, &mut hdlc_output).is_ok() {
                        if let Err(e) = writer.write_all(hdlc_output.as_slice()).await {
                            log::warn!(
                                "stream_iface: write_all failed on {}: {}",
                                iface_address, e
                            );
                            break;
                        }
                        if let Err(e) = writer.flush().await {
                            log::warn!(
                                "stream_iface: flush failed on {}: {}",
                                iface_address, e
                            );
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
