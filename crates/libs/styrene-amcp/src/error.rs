use std::io;

#[derive(Debug, thiserror::Error)]
pub enum AmcpError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("server error {code}: {message}")]
    Server { code: u16, message: String },
}

pub type Result<T> = std::result::Result<T, AmcpError>;
