//! Interface Access Codes (IFAC) — per-interface packet authentication.
//!
//! IFAC is Reticulum's mechanism for authenticating packets on a per-interface
//! basis. An interface configured with IFAC requires that all packets carry a
//! valid token, preventing unauthenticated nodes from injecting traffic.
//!
//! # Wire format (IFAC-enabled packet)
//!
//! ```text
//! Byte 0:      flags | 0x80         (IFAC bit forced set; also XOR-masked)
//! Byte 1:      hops                 (XOR-masked)
//! Bytes 2..N:  ifac_token           (NOT masked — the access code itself)
//! Bytes N+1..: rest of inner packet (XOR-masked)
//! ```
//!
//! Where N = 2 + ifac_size and the XOR mask is:
//!
//! ```text
//! mask = HKDF-SHA256(ikm=ifac_token, salt=interface.ifac_key, length=len(wire_bytes))
//! ```
//!
//! The IFAC token is the last `ifac_size` bytes of an Ed25519 signature:
//!
//! ```text
//! ifac_token = Ed25519.sign(inner_packet_bytes)[-ifac_size..]
//! ```
//!
//! where `inner_packet_bytes` has the IFAC flag cleared and IFAC bytes absent.
//!
//! # Key insight: IFAC is symmetric, not public-key
//!
//! Both sender and receiver hold the SAME private key (shared secret). The
//! receiver verifies by re-signing the stripped inner packet and comparing.
//! This is a MAC-like construction — not a PKI scheme.
//!
//! # Multi-hop correctness
//!
//! IFAC is applied at the **interface boundary**, not the transport layer:
//!   - Inbound: strip IFAC, verify, pass clean inner packet to transport
//!   - Outbound: transport gives clean packet, add IFAC for the egress interface
//!
//! Each forwarding hop re-applies IFAC for its own outbound interface, so the
//! token is always fresh relative to the current hops count. This is why the
//! hops byte is masked — to prevent passive observers from learning routing
//! topology from the unencrypted hop counter.

use hkdf::Hkdf;
use sha2::Sha256;

use crate::identity::PrivateIdentity;

/// Default IFAC token length in bytes. RNS default is 8; configurable 1–64.
pub const DEFAULT_IFAC_SIZE: usize = 8;

/// Per-interface IFAC configuration (shared secret between all nodes on the interface).
#[derive(Clone)]
pub struct IfacConfig {
    /// HKDF salt — derived from the raw interface secret during setup.
    pub key: Vec<u8>,
    /// Ed25519 identity whose private key is shared by all authorized nodes.
    /// Used to sign (outbound) and verify by re-signing (inbound).
    pub identity: PrivateIdentity,
    /// Number of bytes to use as the IFAC token (1–64, default 8).
    pub ifac_size: usize,
}

impl IfacConfig {
    pub fn new(key: Vec<u8>, identity: PrivateIdentity, ifac_size: usize) -> Self {
        Self { key, identity, ifac_size }
    }
}

/// Derive the XOR mask for IFAC masking/unmasking.
///
/// `mask = HKDF-SHA256(ikm=ifac_token, salt=interface_key, length=needed)`
fn derive_mask(ifac_token: &[u8], key: &[u8], length: usize) -> Vec<u8> {
    let hk = Hkdf::<Sha256>::new(Some(key), ifac_token);
    let mut mask = vec![0u8; length];
    hk.expand(&[], &mut mask).expect("HKDF expand length within bounds");
    mask
}

/// Wrap a raw outbound packet with IFAC authentication.
///
/// `raw` must be a valid serialized RNS packet WITHOUT the IFAC flag set.
/// Returns the masked, IFAC-wrapped bytes ready for transmission.
///
/// Mirrors Python `Transport.transmit` IFAC branch.
pub fn ifac_wrap(raw: &[u8], config: &IfacConfig) -> Vec<u8> {
    // Sign the inner packet; take last ifac_size bytes as the token.
    let sig = config.identity.sign(raw);
    let sig_bytes = sig.to_bytes();
    let ifac_token = &sig_bytes[sig_bytes.len() - config.ifac_size..];

    let wire_len = raw.len() + config.ifac_size;
    let mask = derive_mask(ifac_token, &config.key, wire_len);

    // Assemble: (flags | 0x80) + hops + ifac_token + raw[2..]
    let mut assembled = Vec::with_capacity(wire_len);
    assembled.push(raw[0] | 0x80);
    assembled.push(raw[1]);
    assembled.extend_from_slice(ifac_token);
    assembled.extend_from_slice(&raw[2..]);

    // XOR-mask with per-byte rules:
    //   i=0:              mask, but keep IFAC flag set
    //   i=1:              mask (hops)
    //   i=2..ifac_size+1: no mask (IFAC token must be transmitted as-is)
    //   i>ifac_size+1:    mask (payload)
    assembled
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if i == 0 {
                (b ^ mask[i]) | 0x80
            } else if i == 1 || i > config.ifac_size + 1 {
                b ^ mask[i]
            } else {
                b // IFAC token bytes: unmask
            }
        })
        .collect()
}

/// Strip and verify IFAC authentication from an inbound raw packet.
///
/// `raw` must be the bytes as received from the wire (IFAC flag set, masked).
/// Returns the inner packet bytes (IFAC stripped, flag cleared) on success,
/// or `None` if validation fails or the packet is malformed.
///
/// Mirrors Python `Transport.inbound` IFAC branch.
pub fn ifac_unwrap(raw: &[u8], config: &IfacConfig) -> Option<Vec<u8>> {
    if raw.len() <= 2 + config.ifac_size {
        return None;
    }
    if raw[0] & 0x80 == 0 {
        return None; // IFAC flag not set
    }

    // The IFAC token occupies bytes 2..2+ifac_size and is NOT masked.
    let ifac_token = &raw[2..2 + config.ifac_size];
    let mask = derive_mask(ifac_token, &config.key, raw.len());

    // Unmask:
    //   i=0,1: unmask header bytes
    //   i=2..=ifac_size+1: IFAC token passes through
    //   i>ifac_size+1: unmask payload
    let unmasked: Vec<u8> = raw
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if i <= 1 || i > config.ifac_size + 1 {
                b ^ mask[i]
            } else {
                b
            }
        })
        .collect();

    // Clear the IFAC flag in the unmasked header and strip IFAC token bytes.
    let inner_header = [unmasked[0] & 0x7f, unmasked[1]];
    let mut inner = Vec::with_capacity(raw.len() - config.ifac_size);
    inner.extend_from_slice(&inner_header);
    inner.extend_from_slice(&unmasked[2 + config.ifac_size..]);

    // Verify by re-signing: expected = sign(inner)[-ifac_size..]
    let expected_sig = config.identity.sign(&inner);
    let expected_bytes = expected_sig.to_bytes();
    let expected_token = &expected_bytes[expected_bytes.len() - config.ifac_size..];

    if ifac_token == expected_token {
        Some(inner)
    } else {
        log::debug!("ifac: token mismatch — dropping packet");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::PrivateIdentity;

    fn make_config(ifac_size: usize) -> IfacConfig {
        let identity = PrivateIdentity::new_from_rand(rand_core::OsRng);
        IfacConfig::new(b"test-iface-key-32bytes-long-xxxx".to_vec(), identity, ifac_size)
    }

    /// A minimal but valid-looking raw packet (header + dest + context + data).
    fn test_inner() -> Vec<u8> {
        let mut p = vec![0x00u8, 0x00]; // flags, hops
        p.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03, 0x04,
                               0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c]); // dest
        p.push(0x00); // context
        p.extend_from_slice(b"hello ifac"); // data
        p
    }

    #[test]
    fn round_trip_default_size() {
        let config = make_config(DEFAULT_IFAC_SIZE);
        let inner = test_inner();
        let wrapped = ifac_wrap(&inner, &config);

        assert_eq!(wrapped.len(), inner.len() + DEFAULT_IFAC_SIZE);
        assert_eq!(wrapped[0] & 0x80, 0x80, "IFAC flag must be set in wire bytes");

        let recovered = ifac_unwrap(&wrapped, &config).expect("round-trip must succeed");
        assert_eq!(recovered, inner);
    }

    #[test]
    fn round_trip_small_ifac_size() {
        let config = make_config(4);
        let inner = test_inner();
        let wrapped = ifac_wrap(&inner, &config);
        let recovered = ifac_unwrap(&wrapped, &config).expect("round-trip with ifac_size=4");
        assert_eq!(recovered, inner);
    }

    #[test]
    fn tampered_payload_rejected() {
        let config = make_config(DEFAULT_IFAC_SIZE);
        let mut wrapped = ifac_wrap(&test_inner(), &config);
        *wrapped.last_mut().unwrap() ^= 0x01;
        assert!(ifac_unwrap(&wrapped, &config).is_none(), "tampered packet must be rejected");
    }

    #[test]
    fn wrong_key_rejected() {
        let config1 = make_config(DEFAULT_IFAC_SIZE);
        let mut config2 = config1.clone();
        config2.key = b"different-key-not-the-same-xxxxx".to_vec();
        let wrapped = ifac_wrap(&test_inner(), &config1);
        assert!(ifac_unwrap(&wrapped, &config2).is_none(), "wrong key must reject");
    }

    #[test]
    fn wrong_identity_rejected() {
        let config1 = make_config(DEFAULT_IFAC_SIZE);
        let config2 = IfacConfig::new(
            config1.key.clone(),
            PrivateIdentity::new_from_rand(rand_core::OsRng), // different identity
            DEFAULT_IFAC_SIZE,
        );
        let wrapped = ifac_wrap(&test_inner(), &config1);
        assert!(ifac_unwrap(&wrapped, &config2).is_none(), "wrong identity must reject");
    }

    #[test]
    fn hops_masked_in_wire_but_preserved_in_inner() {
        let config = make_config(DEFAULT_IFAC_SIZE);
        let mut inner = test_inner();
        inner[1] = 0x05; // set a non-zero hop count
        let wrapped = ifac_wrap(&inner, &config);
        let recovered = ifac_unwrap(&wrapped, &config).unwrap();
        assert_eq!(recovered[1], 0x05, "hops must survive round-trip");
    }

    #[test]
    fn too_short_rejected() {
        let config = make_config(DEFAULT_IFAC_SIZE);
        let short = vec![0x80u8, 0x00, 0x01, 0x02]; // less than 2 + ifac_size
        assert!(ifac_unwrap(&short, &config).is_none());
    }

    #[test]
    fn missing_ifac_flag_rejected() {
        let config = make_config(DEFAULT_IFAC_SIZE);
        let mut wrapped = ifac_wrap(&test_inner(), &config);
        // Force-clear the IFAC flag in the wire encoding.
        // Note: after masking, byte 0 has bit 7 forced to 1; clear it manually.
        wrapped[0] &= 0x7f;
        assert!(ifac_unwrap(&wrapped, &config).is_none());
    }
}
