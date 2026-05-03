//! Serial / KISS interface — RNode, RP2040, ESP32 LoRa hardware.
//!
//! Uses `tokio-serial` for async serial I/O. The HDLC framing pipeline is
//! provided by `stream_iface::{run_hdlc_rx_loop, run_hdlc_tx_loop}` — no
//! duplication needed.
//!
//! # KISS framing
//!
//! When `use_kiss = true`, the serial stream is wrapped in KISS byte-stuffing
//! (FEND/FESC) before HDLC. This is required for TNC devices (RNode, LoRa
//! hardware). Direct serial connections (no TNC) can set `use_kiss = false`
//! to use raw HDLC framing.
//!
//! Depends on feature `serial` (adds `tokio-serial` to the crate).

#[cfg(feature = "serial")]
#[allow(clippy::unwrap_used)] // Conventional for tokio driver mutex locks
mod inner {
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};

    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio_serial::SerialPortBuilderExt;
    use tokio_util::sync::CancellationToken;

    use super::super::kiss::{kiss_encode, KissDecoder};
    use super::super::stream_iface::{run_hdlc_rx_loop, run_hdlc_tx_loop};
    use crate::transport::iface::{Interface, InterfaceContext};

    /// AsyncRead adapter that strips KISS framing from the underlying reader.
    ///
    /// Reads raw bytes, feeds them through `KissDecoder`, and emits decoded
    /// data frames as a contiguous byte stream for the HDLC layer.
    struct KissReader<R> {
        inner: R,
        decoder: KissDecoder,
        pending: Vec<u8>,
        pending_offset: usize,
    }

    impl<R> KissReader<R> {
        fn new(inner: R) -> Self {
            Self { inner, decoder: KissDecoder::new(), pending: Vec::new(), pending_offset: 0 }
        }

        /// Drain pending decoded data into the caller's buffer.
        /// Returns true if any bytes were copied.
        fn drain_pending(&mut self, buf: &mut ReadBuf<'_>) -> bool {
            if self.pending_offset >= self.pending.len() {
                return false;
            }
            let available = &self.pending[self.pending_offset..];
            let to_copy = available.len().min(buf.remaining());
            if to_copy == 0 {
                return false;
            }
            buf.put_slice(&available[..to_copy]);
            self.pending_offset += to_copy;
            if self.pending_offset >= self.pending.len() {
                self.pending.clear();
                self.pending_offset = 0;
            }
            true
        }
    }

    impl<R: AsyncRead + Unpin> AsyncRead for KissReader<R> {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let this = self.get_mut();

            // Drain any pending decoded data first.
            if this.drain_pending(buf) {
                return Poll::Ready(Ok(()));
            }

            // Loop: read from the inner stream until we get a complete KISS
            // frame or the inner reader returns Pending. This avoids busy-spin
            // — we only return Pending when the inner reader itself is Pending,
            // which means its waker is properly registered.
            loop {
                let mut raw_buf = [0u8; 2048];
                let mut raw_read_buf = ReadBuf::new(&mut raw_buf);
                match Pin::new(&mut this.inner).poll_read(cx, &mut raw_read_buf) {
                    Poll::Ready(Ok(())) => {
                        let n = raw_read_buf.filled().len();
                        if n == 0 {
                            // EOF — drain any remaining decoded data before
                            // signalling end-of-stream to the caller.
                            this.drain_pending(buf);
                            return Poll::Ready(Ok(()));
                        }
                        this.decoder.feed(&raw_buf[..n]);

                        // Collect all decoded frames into pending buffer.
                        while let Some(frame) = this.decoder.take_frame() {
                            this.pending.extend_from_slice(&frame);
                        }

                        if this.drain_pending(buf) {
                            return Poll::Ready(Ok(()));
                        }
                        // No complete frame yet — loop to read more from inner.
                        // The inner reader returned Ready, so its waker is NOT
                        // registered. We must poll it again to either get more
                        // data or have it register the waker via Pending.
                    }
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                }
            }
        }
    }

    /// AsyncWrite adapter that wraps outbound data in KISS framing.
    ///
    /// Buffers KISS-encoded output internally and drains it across multiple
    /// `poll_write` calls to handle partial writes from the inner writer.
    struct KissWriter<W> {
        inner: W,
        /// Buffered KISS-encoded bytes waiting to be written.
        outbuf: Vec<u8>,
        /// How many bytes of `outbuf` have been flushed to the inner writer.
        out_offset: usize,
    }

    impl<W> KissWriter<W> {
        fn new(inner: W) -> Self {
            Self { inner, outbuf: Vec::new(), out_offset: 0 }
        }
    }

    impl<W: AsyncWrite + Unpin> AsyncWrite for KissWriter<W> {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            let this = self.get_mut();

            // If we have buffered KISS bytes from a previous partial write,
            // drain them first before accepting new input.
            while this.out_offset < this.outbuf.len() {
                let remaining = &this.outbuf[this.out_offset..];
                match Pin::new(&mut this.inner).poll_write(cx, remaining) {
                    Poll::Ready(Ok(0)) => {
                        // Zero-byte write — inner writer is stalled.
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                    Poll::Ready(Ok(n)) => {
                        this.out_offset += n;
                    }
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                }
            }
            this.outbuf.clear();
            this.out_offset = 0;

            // Encode new data and attempt to write all of it.
            let kissed = kiss_encode(buf);
            match Pin::new(&mut this.inner).poll_write(cx, &kissed) {
                Poll::Ready(Ok(n)) => {
                    if n < kissed.len() {
                        // Partial write — buffer the remainder for next call.
                        this.outbuf = kissed;
                        this.out_offset = n;
                    }
                    // Report full input consumed: the KISS-encoded bytes are
                    // either fully written or buffered for subsequent flushes.
                    Poll::Ready(Ok(buf.len()))
                }
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            }
        }

        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            let this = self.get_mut();
            // Drain any buffered KISS bytes before flushing the inner writer.
            while this.out_offset < this.outbuf.len() {
                let remaining = &this.outbuf[this.out_offset..];
                match Pin::new(&mut this.inner).poll_write(cx, remaining) {
                    Poll::Ready(Ok(0)) => {
                        // Zero-byte write — inner writer is stalled.
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                    Poll::Ready(Ok(n)) => {
                        this.out_offset += n;
                    }
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                }
            }
            this.outbuf.clear();
            this.out_offset = 0;
            Pin::new(&mut this.inner).poll_flush(cx)
        }

        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
        }
    }

    pub struct SerialInterface {
        path: String,
        baud_rate: u32,
        use_kiss: bool,
    }

    impl SerialInterface {
        /// Create a serial interface with raw HDLC framing (no KISS).
        pub fn new(path: impl Into<String>, baud_rate: u32) -> Self {
            Self { path: path.into(), baud_rate, use_kiss: false }
        }

        /// Create a serial interface with KISS framing (for TNC/RNode devices).
        pub fn new_kiss(path: impl Into<String>, baud_rate: u32) -> Self {
            Self { path: path.into(), baud_rate, use_kiss: true }
        }

        pub async fn spawn(context: InterfaceContext<Self>) {
            let iface_stop = context.channel.stop.clone();
            let (path, baud_rate, use_kiss) = {
                let inner = context.inner.lock().unwrap();
                (inner.path.clone(), inner.baud_rate, inner.use_kiss)
            };
            let iface_address = context.channel.address;
            let (rx_channel, tx_channel) = context.channel.split();
            let tx_channel = Arc::new(tokio::sync::Mutex::new(tx_channel));

            loop {
                if context.cancel.is_cancelled() {
                    break;
                }

                let port = tokio_serial::new(&path, baud_rate).open_native_async();

                match port {
                    Err(e) => {
                        log::warn!("serial: failed to open {}: {}", path, e);
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        continue;
                    }
                    Ok(port) => {
                        let mode = if use_kiss { "KISS+HDLC" } else { "HDLC" };
                        log::info!("serial: opened {} @ {}bps ({})", path, baud_rate, mode);
                        let stop = CancellationToken::new();
                        let (read_half, write_half) = tokio::io::split(port);

                        if use_kiss {
                            let rx_task = tokio::spawn(run_hdlc_rx_loop(
                                KissReader::new(read_half),
                                rx_channel.clone(),
                                iface_address,
                                context.cancel.clone(),
                                stop.clone(),
                                context.ifac.clone(),
                            ));
                            let tx_task = tokio::spawn(run_hdlc_tx_loop(
                                KissWriter::new(write_half),
                                tx_channel.clone(),
                                iface_address,
                                context.cancel.clone(),
                                stop,
                                context.ifac.clone(),
                            ));
                            tx_task.await.ok();
                            rx_task.await.ok();
                        } else {
                            let rx_task = tokio::spawn(run_hdlc_rx_loop(
                                read_half,
                                rx_channel.clone(),
                                iface_address,
                                context.cancel.clone(),
                                stop.clone(),
                                context.ifac.clone(),
                            ));
                            let tx_task = tokio::spawn(run_hdlc_tx_loop(
                                write_half,
                                tx_channel.clone(),
                                iface_address,
                                context.cancel.clone(),
                                stop,
                                context.ifac.clone(),
                            ));
                            tx_task.await.ok();
                            rx_task.await.ok();
                        }
                        log::info!("serial: closed {}", path);
                    }
                }
            }

            iface_stop.cancel();
        }
    }

    impl Interface for SerialInterface {
        fn mtu() -> usize {
            256
        } // LoRa typical MTU
    }
}

#[cfg(feature = "serial")]
pub use inner::SerialInterface;
