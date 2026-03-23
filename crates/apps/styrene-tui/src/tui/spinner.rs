//! Spinner verbs — mesh-themed action phrases during daemon operations.
//!
//! Rotates through themed verb phrases on each announce, link event,
//! or background operation. Displayed in the compose area while working.

use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Get the next spinner verb. Advances the counter each call.
pub fn next_verb() -> &'static str {
    let idx = COUNTER.fetch_add(1, Ordering::Relaxed) % VERBS.len();
    VERBS[idx]
}

/// Seed the starting position (e.g. from process start time).
pub fn seed(value: usize) {
    COUNTER.store(value % VERBS.len(), Ordering::Relaxed);
}

const VERBS: &[&str] = &[
    // ═══ Reticulum mesh operations ═══
    "Propagating announces",
    "Resolving path to destination",
    "Establishing encrypted link",
    "Exchanging ratchet keys",
    "Verifying destination hash",
    "Synchronising with propagation node",
    "Ingesting LXMF messages",
    "Computing proof-of-work stamp",
    "Validating announce signature",
    "Probing transport interfaces",
    "Broadcasting path request",
    "Awaiting link activation proof",
    "Decrypting inbound packet",
    "Routing via next hop",
    "Updating announce table",
    "Flushing packet cache",
    "Querying known destinations",
    "Peering with LXMF node",
    "Fetching from propagation store",
    "Rebuilding path table",
    "Sending keepalive",
    "Scanning for new interfaces",
    "Verifying receipt proof",
    "Allocating resource transfer",
    "Performing link handshake",
    "Discovering mesh topology",
    "Checking announce rate limits",
    "Validating path response tag",
    "Opening channel buffer",
    "Negotiating channel window",
    "Delivering to destination",
    "Compiling announce retransmit",
    "Selecting outbound propagation node",
    "Encoding LXMF wire message",
    "Checking stamp cost",
    "Unwrapping resource advertisement",
    "Waiting for delivery receipt",
    "Routing over BATMAN-adv fabric",
    "Syncing fleet configuration",
    "Registering local destination",
];
