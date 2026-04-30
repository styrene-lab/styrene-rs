// Upstream code — unwrap on mutex locks and bind results is conventional in tokio drivers
#![allow(clippy::unwrap_used)]

use alloc::string::String;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::transport::error::RnsError;

use super::tcp_client::TcpClient;
use super::{Interface, InterfaceContext, InterfaceManager};

pub struct TcpServer {
    addr: String,
    iface_manager: Arc<tokio::sync::Mutex<InterfaceManager>>,
    /// Sender for the actual bound address — set after `TcpListener::bind`.
    /// Enables callers to discover the real port when binding to `:0`.
    bound_addr_tx: watch::Sender<Option<SocketAddr>>,
}

impl TcpServer {
    pub fn new<T: Into<String>>(
        addr: T,
        iface_manager: Arc<tokio::sync::Mutex<InterfaceManager>>,
    ) -> (Self, watch::Receiver<Option<SocketAddr>>) {
        let (bound_addr_tx, bound_addr_rx) = watch::channel(None);
        (
            Self { addr: addr.into(), iface_manager, bound_addr_tx },
            bound_addr_rx,
        )
    }

    /// Spawn the TCP server. Accepted client connections inherit the server's
    /// IFAC configuration (if any) so that all clients on this listener enforce
    /// the same interface authentication.
    pub async fn spawn(context: InterfaceContext<Self>) {
        let addr = { context.inner.lock().unwrap().addr.clone() };

        let iface_manager = { context.inner.lock().unwrap().iface_manager.clone() };
        let server_ifac = context.ifac.clone();

        let (_, tx_channel) = context.channel.split();
        let tx_channel = Arc::new(tokio::sync::Mutex::new(tx_channel));

        loop {
            if context.cancel.is_cancelled() {
                break;
            }

            let listener =
                TcpListener::bind(addr.clone()).await.map_err(|_| RnsError::ConnectionError);

            if listener.is_err() {
                log::warn!("tcp_server: couldn't bind to <{}>", addr);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }

            let listener = listener.unwrap();

            // Publish the actual bound address so callers can discover the
            // real port when binding to `:0` (ephemeral port).
            if let Ok(local_addr) = listener.local_addr() {
                let _ = context.inner.lock().unwrap().bound_addr_tx.send(Some(local_addr));
                log::info!("tcp_server: listen on <{}>", local_addr);
            } else {
                log::info!("tcp_server: listen on <{}>", addr);
            }

            let tx_task = {
                let cancel = context.cancel.clone();
                let tx_channel = tx_channel.clone();

                tokio::spawn(async move {
                    loop {
                        if cancel.is_cancelled() {
                            break;
                        }

                        let mut tx_channel = tx_channel.lock().await;

                        tokio::select! {
                            _ = cancel.cancelled() => {
                                break;
                            }
                            // Skip all tx messages
                            _ = tx_channel.recv() => {}
                        }
                    }
                })
            };

            let cancel = context.cancel.clone();

            loop {
                if cancel.is_cancelled() {
                    break;
                }

                tokio::select! {
                    _ = cancel.cancelled() => {
                        break;
                    }

                    client = listener.accept() => {
                        if let Ok(client) = client {
                            log::info!(
                                "tcp_server: new client <{}> connected to <{}>",
                                client.1,
                                addr
                            );

                            let mut iface_manager = iface_manager.lock().await;

                            iface_manager.spawn_with_ifac(
                                TcpClient::new_from_stream(client.1.to_string(), client.0),
                                TcpClient::spawn,
                                server_ifac.clone(),
                            );
                        }
                    }
                }
            }

            let _ = tokio::join!(tx_task);
        }
    }
}

impl Interface for TcpServer {
    fn mtu() -> usize {
        2048
    }
}
