#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RnsError {
    OutOfMemory,
    InvalidArgument,
    IncorrectSignature,
    IncorrectHash,
    CryptoError,
    PacketError,
    ConnectionError,
    /// No wall-clock time is available for a timestamp-dependent operation.
    /// Without `std` the embedding must supply Unix time first.
    TimeUnavailable,
}
