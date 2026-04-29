use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::TcpStream;
use tracing::{debug, trace, warn};

use crate::command::AmcpCommand;
use crate::error::{AmcpError, Result};
use crate::response::AmcpResponse;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct AmcpClient {
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: BufWriter<tokio::net::tcp::OwnedWriteHalf>,
    timeout: Duration,
}

impl AmcpClient {
    pub async fn connect(addr: &str) -> Result<Self> {
        Self::connect_with_timeout(addr, DEFAULT_TIMEOUT).await
    }

    pub async fn connect_with_timeout(addr: &str, timeout: Duration) -> Result<Self> {
        let stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| AmcpError::Protocol(format!("connection timed out: {addr}")))?
            .map_err(AmcpError::Io)?;

        debug!("connected to CasparCG at {addr}");
        let (read_half, write_half) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(read_half),
            writer: BufWriter::new(write_half),
            timeout,
        })
    }

    pub async fn send(&mut self, cmd: AmcpCommand) -> Result<AmcpResponse> {
        let wire = format!("{cmd}\r\n");
        trace!(">> {}", wire.trim());
        self.writer.write_all(wire.as_bytes()).await?;
        self.writer.flush().await?;
        self.read_response().await
    }

    async fn read_response(&mut self) -> Result<AmcpResponse> {
        // Read status line with timeout
        let status_line = self.read_line_timeout().await?;
        let status_trimmed = status_line.trim();

        let code: u16 = status_trimmed
            .split_whitespace()
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                AmcpError::Protocol(format!("invalid status line: {status_trimmed}"))
            })?;

        // Read body based on response code semantics:
        //   200 = multiline body, terminated by blank line
        //   201 = single line of data
        //   202+ = no body
        let body = if AmcpResponse::has_multiline_body(code) {
            self.read_multiline_body().await?
        } else if AmcpResponse::has_single_line_body(code) {
            let line = self.read_line_timeout().await?;
            line.trim().to_owned()
        } else {
            String::new()
        };

        trace!("<< {code} (body: {} bytes)", body.len());

        let resp = AmcpResponse { code, body };
        if resp.is_success() {
            Ok(resp)
        } else {
            Err(AmcpError::Server {
                code,
                message: format!("{}: {}", status_trimmed, resp.body),
            })
        }
    }

    async fn read_line_timeout(&mut self) -> Result<String> {
        let mut buf = String::new();
        let result = tokio::time::timeout(self.timeout, self.reader.read_line(&mut buf)).await;

        match result {
            Ok(Ok(0)) => Err(AmcpError::Protocol("connection closed".to_owned())),
            Ok(Ok(_)) => Ok(buf),
            Ok(Err(e)) => Err(AmcpError::Io(e)),
            Err(_) => {
                warn!("read timed out after {:?}", self.timeout);
                Err(AmcpError::Protocol(format!(
                    "read timed out after {:?}",
                    self.timeout
                )))
            }
        }
    }

    async fn read_multiline_body(&mut self) -> Result<String> {
        let mut body = String::new();
        loop {
            let line = self.read_line_timeout().await?;
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                break;
            }
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(trimmed);
        }
        Ok(body)
    }
}
