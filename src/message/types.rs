#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageState {
    Generating,
    Outbound,
    Sending,
    Sent,
    Delivered,
    Rejected,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageMethod {
    Unknown,
    Packet,
    Resource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMethod {
    Opportunistic,
    Direct,
    Propagated,
    Paper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnverifiedReason {
    SourceUnknown,
    SignatureInvalid,
}
