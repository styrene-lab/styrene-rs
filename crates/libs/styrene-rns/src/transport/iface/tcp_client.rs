// Upstream code — unwrap on mutex locks and task joins is conventional in tokio drivers
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::OnceLock;

use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use crate::transport::error::RnsError;

use alloc::string::String;

use super::stream_iface::{run_hdlc_rx_loop, run_hdlc_tx_loop};
use super::{Interface, InterfaceContext};

fn tx_diag_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("RETICULUMD_DIAGNOSTICS")
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

        let (rx_channel, tx_channel) = context.channel.split();
        let tx_channel = Arc::new(tokio::sync::Mutex::new(tx_channel));

        let mut running = true;
        loop {
            if !running || context.cancel.is_cancelled() {
                break;
            }

            let stream = match stream.take() {
                Some(s) => {
                    running = false;
                    Ok(s)
                }
                None => TcpStream::connect(addr.clone())
                    .await
                    .map_err(|_| RnsError::ConnectionError),
            };

            if stream.is_err() {
                log::info!("tcp_client: couldn't connect to <{}>", addr);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }

            let cancel = context.cancel.clone();
            let stop = CancellationToken::new();
            let stream = stream.unwrap();
            let (read_half, write_half) = stream.into_split();

            log::info!("tcp_client connected to <{}>", addr);
            if tx_diag_enabled() {
                eprintln!("[tp-diag] tcp_client connected iface={}", iface_address);
            }

            let rx_task = {
                let cancel = cancel.clone();
                let stop = stop.clone();
                let rx_channel = rx_channel.clone();
                tokio::spawn(run_hdlc_rx_loop(
                    read_half,
                    rx_channel,
                    iface_address,
                    cancel,
                    stop,
                ))
            };

            let tx_task = {
                let cancel = cancel.clone();
                let tx_channel = tx_channel.clone();
                tokio::spawn(run_hdlc_tx_loop(
                    write_half,
                    tx_channel,
                    iface_address,
                    cancel,
                    stop.clone(),
                ))
            };

            tx_task.await.unwrap();
            rx_task.await.unwrap();

            log::info!("tcp_client: disconnected from <{}>", addr);
        }

        iface_stop.cancel();
    }
}

impl Interface for TcpClient {
    fn mtu() -> usize {
        2048
    }
}
