// Upstream code — unwrap on mutex locks and task joins is conventional in tokio drivers
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use crate::transport::error::RnsError;

use alloc::string::String;

use super::stream_iface::{RxAdmission, run_hdlc_rx_loop, run_hdlc_tx_loop};
use super::{
    Interface, InterfaceContext, InterfaceDescriptor, InterfaceEndpoint, InterfaceKind,
    InterfaceMode, InterfaceState,
};

const CONNECT_RETRY_DELAY: Duration = Duration::from_secs(5);
const SOCKET_KEEPALIVE_IDLE: Duration = Duration::from_secs(10);
const SOCKET_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
const SOCKET_KEEPALIVE_RETRIES: u32 = 3;

fn socket_keepalive() -> socket2::TcpKeepalive {
    socket2::TcpKeepalive::new()
        .with_time(SOCKET_KEEPALIVE_IDLE)
        .with_interval(SOCKET_KEEPALIVE_INTERVAL)
        .with_retries(SOCKET_KEEPALIVE_RETRIES)
}

fn configure_socket_liveness(stream: &TcpStream) -> std::io::Result<()> {
    socket2::SockRef::from(stream).set_tcp_keepalive(&socket_keepalive())
}

fn tx_diag_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("STYRENED_DIAGNOSTICS")
            .or_else(|_| std::env::var("RETICULUMD_DIAGNOSTICS"))
            .or_else(|_| std::env::var("RETICULUM_TRANSPORT_DIAGNOSTICS"))
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on" | "debug"
                )
            })
            .unwrap_or(false)
    })
}

pub struct TcpClient {
    addr: String,
    stream: Option<TcpStream>,
}

impl TcpClient {
    pub fn new<T: Into<String>>(addr: T) -> Self {
        Self { addr: addr.into(), stream: None }
    }

    pub fn new_from_stream<T: Into<String>>(addr: T, stream: TcpStream) -> Self {
        Self { addr: addr.into(), stream: Some(stream) }
    }

    pub async fn spawn(context: InterfaceContext<TcpClient>) {
        let iface_stop = context.channel.stop.clone();
        let addr = { context.inner.lock().unwrap().addr.clone() };
        let iface_address = context.channel.address;
        let mut stream = { context.inner.lock().unwrap().stream.take() };
        let accepted = stream.is_some();
        let runtime = context.runtime.clone();

        let (rx_channel, tx_channel) = context.channel.split();
        let tx_channel = Arc::new(tokio::sync::Mutex::new(tx_channel));

        let mut running = true;
        loop {
            if !running || context.cancel.is_cancelled() || iface_stop.is_cancelled() {
                break;
            }

            let stream = match stream.take() {
                Some(s) => {
                    running = false;
                    Ok(s)
                }
                None => {
                    runtime.clear_endpoints();
                    runtime.set_state(InterfaceState::Connecting);
                    if tx_diag_enabled() {
                        crate::transport_diagnostic!(
                            "[tp-diag] tcp_client connect_attempt iface={} addr={}",
                            iface_address,
                            addr
                        );
                    }
                    TcpStream::connect(addr.clone()).await.map_err(|_| RnsError::ConnectionError)
                }
            };

            if stream.is_err() {
                runtime.set_state(InterfaceState::Retrying);
                log::info!("tcp_client: couldn't connect to <{}>", addr);
                if tx_diag_enabled() {
                    crate::transport_diagnostic!(
                        "[tp-diag] tcp_client connect_failed iface={} addr={}",
                        iface_address,
                        addr
                    );
                }
                tokio::select! {
                    _ = context.cancel.cancelled() => break,
                    _ = iface_stop.cancelled() => break,
                    _ = tokio::time::sleep(CONNECT_RETRY_DELAY) => {}
                }
                continue;
            }

            let cancel = context.cancel.clone();
            let stop = CancellationToken::new();
            let stream = stream.unwrap();
            if let Ok(local_addr) = stream.local_addr() {
                runtime.set_local_endpoint(InterfaceEndpoint::Socket(local_addr));
            }
            if let Ok(remote_addr) = stream.peer_addr() {
                runtime.set_remote_endpoint(InterfaceEndpoint::Socket(remote_addr));
            }
            runtime.set_state(InterfaceState::Connected);
            if let Err(error) = configure_socket_liveness(&stream) {
                log::warn!("tcp_client: failed to configure keepalive for <{}>: {}", addr, error);
            }
            let (read_half, write_half) = stream.into_split();

            log::info!("tcp_client connected to <{}>", addr);
            if tx_diag_enabled() {
                crate::transport_diagnostic!(
                    "[tp-diag] tcp_client connected iface={}",
                    iface_address
                );
            }

            let rx_task = {
                let cancel = cancel.clone();
                let stop = stop.clone();
                let rx_channel = rx_channel.clone();
                let ifac = context.ifac.clone();
                tokio::spawn(run_hdlc_rx_loop(
                    read_half,
                    rx_channel,
                    iface_address,
                    cancel,
                    stop,
                    RxAdmission::new(Self::mtu(), context.stats.clone()),
                    ifac,
                ))
            };

            let tx_task = {
                let cancel = cancel.clone();
                let tx_channel = tx_channel.clone();
                let ifac = context.ifac.clone();
                tokio::spawn(run_hdlc_tx_loop(
                    write_half,
                    tx_channel,
                    iface_address,
                    cancel,
                    stop.clone(),
                    ifac,
                ))
            };

            tokio::select! {
                _ = iface_stop.cancelled() => {}
                result = rx_task => {
                    if tx_diag_enabled() {
                        crate::transport_diagnostic!(
                            "[tp-diag] tcp_client stream_ended iface={} half=rx join_ok={}",
                            iface_address,
                            result.is_ok()
                        );
                    }
                    if let Err(error) = result {
                        log::warn!("tcp_client: receive task failed for <{}>: {}", addr, error);
                    }
                }
                result = tx_task => {
                    if tx_diag_enabled() {
                        crate::transport_diagnostic!(
                            "[tp-diag] tcp_client stream_ended iface={} half=tx join_ok={}",
                            iface_address,
                            result.is_ok()
                        );
                    }
                    if let Err(error) = result {
                        log::warn!("tcp_client: transmit task failed for <{}>: {}", addr, error);
                    }
                }
            }
            stop.cancel();
            runtime.set_state(if accepted {
                InterfaceState::Closed
            } else {
                InterfaceState::Retrying
            });
            if iface_stop.is_cancelled() {
                break;
            }
            if !accepted {
                runtime.clear_endpoints();
            }

            log::info!("tcp_client: disconnected from <{}>", addr);
            if tx_diag_enabled() {
                crate::transport_diagnostic!(
                    "[tp-diag] tcp_client reconnect_scheduled iface={} delay_ms={}",
                    iface_address,
                    CONNECT_RETRY_DELAY.as_millis()
                );
            }
        }

        iface_stop.cancel();
        runtime.set_state(InterfaceState::Closed);
    }
}

impl Interface for TcpClient {
    fn mtu() -> usize {
        2048
    }

    fn descriptor(&self) -> InterfaceDescriptor {
        InterfaceDescriptor {
            kind: InterfaceKind::TcpClient,
            mode: InterfaceMode::Full,
            local_endpoint: None,
            remote_endpoint: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keepalive_failure_window_fits_recovery_deadline() {
        let failure_window =
            SOCKET_KEEPALIVE_IDLE + SOCKET_KEEPALIVE_INTERVAL * SOCKET_KEEPALIVE_RETRIES;

        assert!(failure_window < Duration::from_secs(60));
        assert_eq!(failure_window, Duration::from_secs(25));
    }
}
