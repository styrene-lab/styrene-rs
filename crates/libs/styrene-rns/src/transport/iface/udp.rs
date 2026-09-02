// Upstream code — unwrap on mutex locks and task joins is conventional in tokio drivers
#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;
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
    InterfaceKind, InterfaceMode, InterfaceState, InterfaceStats,
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

    fn decode_packet(
        raw: &[u8],
        ifac: Option<&IfacConfig>,
    ) -> Result<Option<Packet>, InterfaceDropReason> {
        if raw.is_empty() {
            return Ok(None);
        }
        let inner = if let Some(config) = ifac {
            ifac_unwrap(raw, config).ok_or(InterfaceDropReason::IfacFailure)?
        } else if raw[0] & 0x80 != 0 {
            return Err(InterfaceDropReason::IfacFailure);
        } else {
            raw.to_vec()
        };
        Packet::deserialize(&mut InputBuffer::new(&inner))
            .map(Some)
            .map_err(|_| InterfaceDropReason::MalformedFrame)
    }

    /// Whether a bound socket needs operating-system broadcast permission:
    /// only an IPv4 socket that forwards somewhere, because the forwarding
    /// target may be a broadcast address. Receive-only and IPv6 sockets keep
    /// their default capability.
    fn needs_broadcast(local: &SocketAddr, forward_addr: Option<&str>) -> bool {
        forward_addr.is_some() && local.is_ipv4()
    }

    /// Apply the socket capability the configuration requires before the
    /// first send. Returns whether broadcast permission was enabled.
    fn configure_socket(socket: &UdpSocket, forward_addr: Option<&str>) -> std::io::Result<bool> {
        let local = socket.local_addr()?;
        if !Self::needs_broadcast(&local, forward_addr) {
            return Ok(false);
        }
        socket2::SockRef::from(socket).set_broadcast(true)?;
        Ok(true)
    }

    fn admit_datagram(
        raw: &[u8],
        ifac: Option<&IfacConfig>,
        stats: &InterfaceStats,
    ) -> Option<Packet> {
        match Self::decode_packet(raw, ifac) {
            Ok(packet) => packet,
            Err(reason) => {
                stats.record_drop(reason);
                log::debug!("udp_interface: dropping {:?} input", reason);
                None
            }
        }
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
            if let Err(error) = Self::configure_socket(&socket, forward_addr.as_deref()) {
                runtime.set_state(InterfaceState::Retrying);
                log::warn!(
                    "udp_interface: couldn't enable broadcast on <{}>: {}",
                    bind_addr,
                    error
                );
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
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
                                        if let Some(packet) = Self::admit_datagram(
                                            &rx_buffer[..n],
                                            ifac.as_deref(),
                                            &stats,
                                        ) {
                                                if PACKET_TRACE {
                                                    log::trace!("udp_interface: rx << ({}) {}", iface_address, packet);
                                                }
                                                let _ = rx_channel.send(RxMessage::physical(iface_address, packet, Self::mtu())).await;
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
            ..Default::default()
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
                .expect("non-empty packet")
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
    fn empty_datagrams_are_ignored_without_recording_a_violation() {
        let raw = packet_bytes();
        let wrapped = ifac_wrap(&raw, &ifac_config());
        let stats = InterfaceStats::new();

        assert!(matches!(
            UdpInterface::decode_packet(&wrapped, None),
            Err(InterfaceDropReason::IfacFailure)
        ));
        assert!(UdpInterface::admit_datagram(&[], None, &stats).is_none());
        assert!(UdpInterface::admit_datagram(&raw, None, &stats).is_some());
        assert_eq!(stats.snapshot().violations.malformed_frame, 0);
    }

    #[tokio::test]
    #[ignore = "requires loopback UDP sockets"]
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
        assert_eq!(snapshot.violations.malformed_frame, 0);
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

    fn broadcast_enabled(socket: &UdpSocket) -> bool {
        socket2::SockRef::from(socket).broadcast().expect("read SO_BROADCAST")
    }

    #[test]
    fn broadcast_permission_is_required_only_for_ipv4_forwarding_sockets() {
        let v4: SocketAddr = "127.0.0.1:4242".parse().expect("v4");
        let v6: SocketAddr = "[::1]:4242".parse().expect("v6");
        assert!(UdpInterface::needs_broadcast(&v4, Some("255.255.255.255:4242")));
        assert!(UdpInterface::needs_broadcast(&v4, Some("10.0.0.7:4242")));
        assert!(!UdpInterface::needs_broadcast(&v4, None), "receive-only sockets stay default");
        assert!(!UdpInterface::needs_broadcast(&v6, Some("[ff02::1]:4242")), "IPv6 is unchanged");
        assert!(!UdpInterface::needs_broadcast(&v6, None));
    }

    #[tokio::test]
    #[ignore = "requires loopback UDP sockets"]
    async fn ipv4_forwarding_socket_gains_broadcast_before_use_and_controls_do_not() {
        let forwarding = UdpSocket::bind("127.0.0.1:0").await.expect("bind v4");
        assert!(!broadcast_enabled(&forwarding), "a fresh socket has no broadcast permission");
        assert!(
            UdpInterface::configure_socket(&forwarding, Some("255.255.255.255:4242"))
                .expect("configure forwarding socket")
        );
        assert!(broadcast_enabled(&forwarding));
        forwarding
            .send_to(b"probe", "127.255.255.255:4242")
            .await
            .expect("broadcast datagram is permitted");

        let receive_only = UdpSocket::bind("127.0.0.1:0").await.expect("bind receive-only");
        assert!(
            !UdpInterface::configure_socket(&receive_only, None).expect("configure receive-only")
        );
        assert!(!broadcast_enabled(&receive_only));

        let v6 = UdpSocket::bind("[::1]:0").await.expect("bind v6");
        assert!(!UdpInterface::configure_socket(&v6, Some("[::1]:4242")).expect("configure v6"));
        assert!(!broadcast_enabled(&v6));
    }
}
