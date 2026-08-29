// Upstream code — unwrap on mutex locks and task joins is conventional in tokio drivers
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;

use crate::buffer::{InputBuffer, OutputBuffer};
use crate::packet::Packet;
use crate::serde::Serialize;
use crate::transport::error::RnsError;
use crate::transport::iface::RxMessage;

use super::ifac::{IfacConfig, ifac_unwrap, ifac_wrap};
use super::{
    Interface, InterfaceContext, InterfaceDescriptor, InterfaceDropReason, InterfaceEndpoint,
    InterfaceKind, InterfaceMode, InterfaceState,
};

// UDP trace logging stays on by default for packet-level network bring-up visibility.
const PACKET_TRACE: bool = true;

pub struct UdpInterface {
    bind_addr: String,
    forward_addr: Option<String>,
}

impl UdpInterface {
    pub fn new<T: Into<String>>(bind_addr: T, forward_addr: Option<T>) -> Self {
        Self { bind_addr: bind_addr.into(), forward_addr: forward_addr.map(Into::into) }
    }

    fn decode_packet(raw: &[u8], ifac: Option<&IfacConfig>) -> Result<Packet, InterfaceDropReason> {
        if raw.is_empty() {
            return Err(InterfaceDropReason::MalformedFrame);
        }
        let inner = if let Some(config) = ifac {
            ifac_unwrap(raw, config).ok_or(InterfaceDropReason::IfacFailure)?
        } else if raw[0] & 0x80 != 0 {
            return Err(InterfaceDropReason::IfacFailure);
        } else {
            raw.to_vec()
        };
        Packet::deserialize(&mut InputBuffer::new(&inner))
            .map_err(|_| InterfaceDropReason::MalformedFrame)
    }

    pub async fn spawn(context: InterfaceContext<Self>) {
        let bind_addr = { context.inner.lock().unwrap().bind_addr.clone() };
        let forward_addr = { context.inner.lock().unwrap().forward_addr.clone() };
        let iface_address = context.channel.address;
        let runtime = context.runtime.clone();
        let stats = context.stats.clone();
        let ifac = context.ifac.clone();

        let (rx_channel, tx_channel) = context.channel.split();
        let tx_channel = Arc::new(tokio::sync::Mutex::new(tx_channel));

        loop {
            if context.cancel.is_cancelled() {
                break;
            }

            let socket =
                UdpSocket::bind(bind_addr.clone()).await.map_err(|_| RnsError::ConnectionError);

            if socket.is_err() {
                runtime.set_state(InterfaceState::Retrying);
                log::info!("udp_interface: couldn't bind to <{}>", bind_addr);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }

            let cancel = context.cancel.clone();
            let stop = CancellationToken::new();

            let socket = socket.unwrap();
            if let Ok(local_addr) = socket.local_addr() {
                runtime.set_local_endpoint(InterfaceEndpoint::Socket(local_addr));
            }
            runtime.set_state(InterfaceState::Active);
            let read_socket = Arc::new(socket);
            let write_socket = read_socket.clone();

            log::info!("udp_interface bound to <{}>", bind_addr);

            const BUFFER_SIZE: usize = core::mem::size_of::<Packet>() * 3;

            // Start receive task
            let rx_task = {
                let cancel = cancel.clone();
                let stop = stop.clone();
                let socket = read_socket;
                let rx_channel = rx_channel.clone();
                let stats = stats.clone();
                let ifac = ifac.clone();

                tokio::spawn(async move {
                    loop {
                        let mut rx_buffer = [0u8; BUFFER_SIZE];

                        tokio::select! {
                            _ = cancel.cancelled() => {
                                    break;
                            }
                            _ = stop.cancelled() => {
                                    break;
                            }
                            result = socket.recv_from(&mut rx_buffer) => {
                                match result {
                                    Ok((n, _in_addr)) => {
                                        match Self::decode_packet(&rx_buffer[..n], ifac.as_deref()) {
                                            Ok(packet) => {
                                                if PACKET_TRACE {
                                                    log::trace!("udp_interface: rx << ({}) {}", iface_address, packet);
                                                }
                                                let _ = rx_channel.send(RxMessage::physical(iface_address, packet, Self::mtu())).await;
                                            }
                                            Err(reason) => {
                                                stats.record_drop(reason);
                                                log::debug!("udp_interface: dropping {:?} input", reason);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        log::warn!("udp_interface: connection error {}", e);
                                        stop.cancel();
                                        break;
                                    }
                                }
                            },
                        };
                    }
                })
            };

            if let Some(forward_addr) = forward_addr.clone() {
                // Start transmit task
                let tx_task = {
                    let cancel = cancel.clone();
                    let tx_channel = tx_channel.clone();
                    let socket = write_socket;
                    let ifac = ifac.clone();

                    tokio::spawn(async move {
                        loop {
                            if stop.is_cancelled() {
                                break;
                            }

                            let mut tx_buffer = [0u8; BUFFER_SIZE];

                            let mut tx_channel = tx_channel.lock().await;

                            tokio::select! {
                                _ = cancel.cancelled() => {
                                        break;
                                }
                                _ = stop.cancelled() => {
                                        break;
                                }
                                Some(message) = tx_channel.recv() => {
                                    let packet = message.packet;
                                    if PACKET_TRACE {
                                        log::trace!("udp_interface: tx >> ({}) {}", iface_address, packet);
                                    }
                                    let mut output = OutputBuffer::new(&mut tx_buffer);
                                    if packet.serialize(&mut output).is_ok() {
                                        let wire = if let Some(config) = ifac.as_deref() {
                                            ifac_wrap(output.as_slice(), config)
                                        } else {
                                            output.as_slice().to_vec()
                                        };
                                        let _ = socket.send_to(&wire, &forward_addr).await;
                                    }
                                }
                            };
                        }
                    })
                };
                tx_task.await.unwrap();
            }

            rx_task.await.unwrap();

            log::info!("udp_interface <{}>: closed", bind_addr);
            runtime.set_state(InterfaceState::Retrying);
        }
        runtime.set_state(InterfaceState::Closed);
    }
}

impl Interface for UdpInterface {
    fn mtu() -> usize {
        2048
    }

    fn descriptor(&self) -> InterfaceDescriptor {
        InterfaceDescriptor {
            kind: InterfaceKind::Udp,
            mode: InterfaceMode::Full,
            local_endpoint: None,
            remote_endpoint: None,
        }
    }
}

pub fn encode_frame(data: &[u8]) -> Result<Vec<u8>, RnsError> {
    Ok(data.to_vec())
}

pub fn decode_frame(frame: &[u8]) -> Result<Vec<u8>, RnsError> {
    Ok(frame.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::PrivateIdentity;
    use crate::packet::PacketDataBuffer;
    use crate::transport::iface::{InterfaceManager, TxMessage, TxMessageType};
    use rand_core::OsRng;
    use tokio::time::{Duration, timeout};

    fn packet_bytes() -> Vec<u8> {
        Packet { data: PacketDataBuffer::new_from_slice(b"udp IFAC"), ..Default::default() }
            .to_bytes()
            .expect("valid packet")
    }

    fn ifac_config() -> IfacConfig {
        IfacConfig::new(b"udp-test-key".to_vec(), PrivateIdentity::new_from_rand(OsRng), 8)
    }

    #[test]
    fn configured_ifac_accepts_authenticated_packets_only() {
        let config = ifac_config();
        let raw = packet_bytes();
        let wrapped = ifac_wrap(&raw, &config);

        assert_eq!(
            UdpInterface::decode_packet(&wrapped, Some(&config))
                .expect("authenticated packet")
                .data
                .as_slice(),
            b"udp IFAC"
        );
        assert!(matches!(
            UdpInterface::decode_packet(&raw, Some(&config)),
            Err(InterfaceDropReason::IfacFailure)
        ));
    }

    #[test]
    fn open_interface_rejects_ifac_and_empty_datagrams_without_closing() {
        let raw = packet_bytes();
        let wrapped = ifac_wrap(&raw, &ifac_config());

        assert!(matches!(
            UdpInterface::decode_packet(&wrapped, None),
            Err(InterfaceDropReason::IfacFailure)
        ));
        assert!(matches!(
            UdpInterface::decode_packet(&[], None),
            Err(InterfaceDropReason::MalformedFrame)
        ));
        assert!(UdpInterface::decode_packet(&raw, None).is_ok());
    }

    #[tokio::test]
    async fn udp_worker_survives_empty_datagram_and_wraps_egress_with_ifac() {
        let reservation = UdpSocket::bind("127.0.0.1:0").await.expect("bind reservation");
        let bind_addr = reservation.local_addr().expect("reserved address");
        drop(reservation);
        let peer = UdpSocket::bind("127.0.0.1:0").await.expect("bind peer");
        let peer_addr = peer.local_addr().expect("peer address");
        let sender = UdpSocket::bind("127.0.0.1:0").await.expect("bind sender");
        let config = Arc::new(ifac_config());
        let mut manager = InterfaceManager::new(4);
        let address = manager.spawn_with_ifac(
            UdpInterface::new(bind_addr.to_string(), Some(peer_addr.to_string())),
            UdpInterface::spawn,
            Some(config.clone()),
        );

        timeout(Duration::from_secs(1), async {
            loop {
                if manager
                    .interface_snapshots()
                    .iter()
                    .any(|snapshot| snapshot.state == InterfaceState::Active)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("UDP worker activation");

        sender.send_to(&[], bind_addr).await.expect("empty datagram");
        let authenticated = ifac_wrap(&packet_bytes(), &config);
        sender.send_to(&authenticated, bind_addr).await.expect("authenticated datagram");
        let received = timeout(Duration::from_secs(1), manager.rx_recv.lock().await.recv())
            .await
            .expect("ingress deadline")
            .expect("authenticated ingress");
        assert_eq!(received.packet.data.as_slice(), b"udp IFAC");
        let snapshot = manager.interface_snapshots().remove(0);
        assert_eq!(snapshot.violations.malformed_frame, 1);
        assert_eq!(snapshot.violations.ifac_failure, 0);

        let outbound = Packet {
            data: PacketDataBuffer::new_from_slice(b"authenticated egress"),
            ..Default::default()
        };
        let trace = manager
            .send(TxMessage { tx_type: TxMessageType::Direct(address), packet: outbound })
            .await;
        assert_eq!(trace.sent_ifaces, 1);
        let mut wire = [0_u8; 4096];
        let (wire_len, _) = timeout(Duration::from_secs(1), peer.recv_from(&mut wire))
            .await
            .expect("egress deadline")
            .expect("egress datagram");
        let inner = ifac_unwrap(&wire[..wire_len], &config).expect("valid egress IFAC");
        assert_eq!(
            Packet::from_bytes(&inner).expect("egress packet").data.as_slice(),
            b"authenticated egress"
        );

        manager.shutdown();
        for task in manager.take_tasks() {
            timeout(Duration::from_secs(1), task)
                .await
                .expect("UDP shutdown deadline")
                .expect("UDP worker join");
        }
    }
}
