use core::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    EncodeFailed,
    DecodeFailed,
    InvalidSignature,
    ChunkCountMismatch,
    TooManyChunks,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EncodeFailed       => write!(f, "manifest encode failed"),
            Self::DecodeFailed       => write!(f, "manifest decode failed"),
            Self::InvalidSignature   => write!(f, "manifest signature invalid"),
            Self::ChunkCountMismatch => write!(f, "chunk count does not match chunk_hashes length"),
            Self::TooManyChunks     => write!(f, "chunk count exceeds maximum (256)"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DistributorError {
    StoreError,
    TransportError,
    VerificationFailed { chunk_index: u32 },
    ManifestError(ManifestError),
    ContentTooLarge,
    NoSeedersKnown,
    Incomplete,
}

impl fmt::Display for DistributorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StoreError             => write!(f, "chunk store error"),
            Self::TransportError         => write!(f, "transport error"),
            Self::VerificationFailed { chunk_index }
                                         => write!(f, "chunk {chunk_index} verification failed"),
            Self::ManifestError(e)       => write!(f, "manifest error: {e}"),
            Self::ContentTooLarge        => write!(f, "content exceeds 256-chunk limit for profile"),
            Self::NoSeedersKnown         => write!(f, "no seeders known for this content"),
            Self::Incomplete             => write!(f, "download incomplete"),
        }
    }
}
