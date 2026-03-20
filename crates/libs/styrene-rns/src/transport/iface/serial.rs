//! Serial / KISS interface — RNode, RP2040, ESP32 LoRa hardware.
//!
//! Uses `tokio-serial` for async serial I/O. The HDLC framing pipeline is
//! provided by `stream_iface::{run_hdlc_rx_loop, run_hdlc_tx_loop}` — no
//! duplication needed.
//!
//! # KISS framing (TODO)
//!
//! KISS adds a thin layer on top of the byte stream before HDLC: a FEND/FESC
//! byte-stuffed envelope identifying the command type (0x00 = data frame).
//! KISS framing will be extracted into a `KissCodec` struct parallel to `Hdlc`
//! and the loop functions will accept a generic `FrameCodec` parameter.
//! For now, this module uses raw HDLC framing — correct for direct serial
//! connections where KISS is not required.
//!
//! Depends on feature `serial` (adds `tokio-serial` to the crate).

#[cfg(feature = "serial")]
mod inner {
    use std::sync::Arc;

    use tokio_serial::SerialPortBuilderExt;
    use tokio_util::sync::CancellationToken;

    use crate::transport::iface::{Interface, InterfaceContext};
    use super::super::stream_iface::{run_hdlc_rx_loop, run_hdlc_tx_loop};

    pub struct SerialInterface {
        path: String,
        baud_rate: u32,
    }

    impl SerialInterface {
        pub fn new(path: impl Into<String>, baud_rate: u32) -> Self {
            Self { path: path.into(), baud_rate }
        }

        pub async fn spawn(context: InterfaceContext<Self>) {
            let iface_stop = context.channel.stop.clone();
            let (path, baud_rate) = {
                let inner = context.inner.lock().unwrap();
                (inner.path.clone(), inner.baud_rate)
            };
            let iface_address = context.channel.address;
            let (rx_channel, tx_channel) = context.channel.split();
            let tx_channel = Arc::new(tokio::sync::Mutex::new(tx_channel));

            loop {
                if context.cancel.is_cancelled() { break; }

                let port = tokio_serial::new(&path, baud_rate)
                    .open_native_async();

                match port {
                    Err(e) => {
                        log::warn!("serial: failed to open {}: {}", path, e);
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        continue;
                    }
                    Ok(port) => {
                        log::info!("serial: opened {} @ {}bps", path, baud_rate);
                        let stop = CancellationToken::new();
                        let (read_half, write_half) = tokio::io::split(port);

                        let rx_task = tokio::spawn(run_hdlc_rx_loop(
                            read_half,
                            rx_channel.clone(),
                            iface_address,
                            context.cancel.clone(),
                            stop.clone(),
                        ));
                        let tx_task = tokio::spawn(run_hdlc_tx_loop(
                            write_half,
                            tx_channel.clone(),
                            iface_address,
                            context.cancel.clone(),
                            stop,
                        ));

                        tx_task.await.ok();
                        rx_task.await.ok();
                        log::info!("serial: closed {}", path);
                    }
                }
            }

            iface_stop.cancel();
        }
    }

    impl Interface for SerialInterface {
        fn mtu() -> usize { 256 } // LoRa typical MTU
    }
}

#[cfg(feature = "serial")]
pub use inner::SerialInterface;
