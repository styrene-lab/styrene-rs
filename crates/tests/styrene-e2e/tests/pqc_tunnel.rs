//! Layer 5 — PQC tunnel session and mock tunnel backend.
//!
//! Exercises the PQC handshake state machine between two peers,
//! validates session key derivation, data encryption/decryption,
//! replay detection, authenticated close, and mock tunnel establishment.

use std::net::IpAddr;

use styrene_e2e::tunnel_mock::MockTunnelBackend;
use styrene_tunnel::session::{PqcSession, SessionState};
use styrene_tunnel::traits::{TunnelBackend, TunnelParams};

fn identity_hash(val: u8) -> [u8; 16] {
    [val; 16]
}

// ── PQC Session Handshake ──────────────────────────────────────────────

#[test]
fn pqc_handshake_establishes_shared_key() {
    let mut initiator = PqcSession::new(identity_hash(0xAA));
    let mut responder = PqcSession::new(identity_hash(0xBB));

    let initiate = initiator.initiate().expect("initiate");
    assert_eq!(initiator.state(), SessionState::Initiating);

    let respond = responder.process_initiate(&initiate).expect("process_initiate");
    assert_eq!(responder.state(), SessionState::Responding);

    let confirm = initiator.process_respond(&respond).expect("process_respond");
    assert_eq!(initiator.state(), SessionState::Established);

    responder.process_confirm(&confirm).expect("process_confirm");
    assert_eq!(responder.state(), SessionState::Established);

    // Both sides derive the same session key
    let ikey = initiator.session_key().expect("initiator key");
    let rkey = responder.session_key().expect("responder key");
    assert_eq!(ikey, rkey, "session keys must match");
    assert_ne!(ikey, &[0u8; 32], "session key must not be zero");
}

// ── Bidirectional Data Encryption ──────────────────────────────────────

#[test]
fn pqc_bidirectional_data_encryption() {
    let (mut initiator, mut responder) = establish();

    // Initiator → Responder
    let data1 = initiator.encrypt_data(b"hello from initiator").expect("encrypt");
    let plain1 = responder.decrypt_data(&data1).expect("decrypt");
    assert_eq!(&plain1, b"hello from initiator");

    // Responder → Initiator
    let data2 = responder.encrypt_data(b"hello from responder").expect("encrypt");
    let plain2 = initiator.decrypt_data(&data2).expect("decrypt");
    assert_eq!(&plain2, b"hello from responder");
}

// ── Replay Detection ───────────────────────────────────────────────────

#[test]
fn pqc_replay_detection() {
    let (mut initiator, mut responder) = establish();

    let data = initiator.encrypt_data(b"once").expect("encrypt");
    responder.decrypt_data(&data).expect("first decrypt");

    // Replay the same packet
    let result = responder.decrypt_data(&data);
    assert!(result.is_err(), "replay must be rejected");
}

#[test]
fn pqc_out_of_order_within_window() {
    let (mut initiator, mut responder) = establish();

    let pkt0 = initiator.encrypt_data(b"pkt0").expect("enc 0");
    let pkt1 = initiator.encrypt_data(b"pkt1").expect("enc 1");
    let pkt2 = initiator.encrypt_data(b"pkt2").expect("enc 2");

    // Deliver out of order: 0, 2, 1
    responder.decrypt_data(&pkt0).expect("pkt0");
    responder.decrypt_data(&pkt2).expect("pkt2 (ahead)");
    responder.decrypt_data(&pkt1).expect("pkt1 (within window)");
}

// ── Authenticated Close ────────────────────────────────────────────────

#[test]
fn pqc_authenticated_close() {
    let (mut initiator, mut responder) = establish();

    let close_action = initiator.close(0x00, Some("goodbye".into())).expect("close");
    assert_eq!(initiator.state(), SessionState::Closed);

    match close_action {
        styrene_tunnel::session::CloseAction::Authenticated(data_payload) => {
            let plaintext = responder.decrypt_data(&data_payload).expect("decrypt close");
            let (reason, message) =
                responder.try_authenticated_close(&plaintext).expect("should be close");
            assert_eq!(reason, 0x00);
            assert_eq!(message.as_deref(), Some("goodbye"));
            assert_eq!(responder.state(), SessionState::Closed);
        }
        styrene_tunnel::session::CloseAction::Unauthenticated(_) => {
            panic!("established session must produce authenticated close");
        }
    }
}

// ── Anti-Reflection ────────────────────────────────────────────────────

#[test]
fn pqc_confirmation_not_reflectable() {
    let mut initiator = PqcSession::new(identity_hash(0xAA));
    let mut responder = PqcSession::new(identity_hash(0xBB));

    let initiate = initiator.initiate().expect("initiate");
    let respond = responder.process_initiate(&initiate).expect("process_initiate");
    let confirm = initiator.process_respond(&respond).expect("process_respond");

    // Confirm and respond encrypted_confirms must differ
    assert_ne!(
        respond.encrypted_confirm, confirm.encrypted_confirm,
        "role-bound confirmations must differ"
    );
}

// ── Mock Tunnel Backend ────────────────────────────────────────────────

#[tokio::test]
async fn mock_tunnel_backend_establish_and_rekey() {
    let mock = MockTunnelBackend::new("wireguard");

    // Derive a PSK from a PQC session
    let (initiator, _responder) = establish();
    let session_key = *initiator.session_key().expect("session key");

    let params = TunnelParams {
        peer_identity: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        remote_endpoint: Some("127.0.0.1".parse::<IpAddr>().expect("parse")),
        remote_port: Some(51820),
        psk: session_key,
        peer_x25519_public: None,
        peer_mesh_ip: None,
        mtu: None,
    };

    // Establish
    let tunnel_id = mock.establish(params).await.expect("establish");
    assert_eq!(mock.establish_count(), 1);
    assert_eq!(mock.last_psk(), Some(session_key));

    // Status
    let info = mock.status(&tunnel_id).await.expect("status");
    assert_eq!(info.backend, "wireguard");
    assert_eq!(info.state, styrene_tunnel::traits::TunnelState::Established);

    // Rekey
    let new_psk = [0x42u8; 32];
    mock.rekey(&tunnel_id, &new_psk).await.expect("rekey");
    assert_eq!(mock.rekey_log.lock().expect("lock").len(), 1);

    // List
    let tunnels = mock.list_tunnels().await.expect("list");
    assert_eq!(tunnels.len(), 1);

    // Teardown
    mock.teardown(&tunnel_id).await.expect("teardown");
    let tunnels = mock.list_tunnels().await.expect("list after teardown");
    assert!(tunnels.is_empty());
}

#[tokio::test]
async fn mock_tunnel_backend_unavailable() {
    let mock = MockTunnelBackend::new("strongswan");
    mock.set_available(false);

    assert!(!mock.is_available().await);
}

// ── Helpers ────────────────────────────────────────────────────────────

fn establish() -> (PqcSession, PqcSession) {
    let mut initiator = PqcSession::new(identity_hash(0xAA));
    let mut responder = PqcSession::new(identity_hash(0xBB));

    let initiate = initiator.initiate().expect("initiate");
    let respond = responder.process_initiate(&initiate).expect("process_initiate");
    let confirm = initiator.process_respond(&respond).expect("process_respond");
    responder.process_confirm(&confirm).expect("process_confirm");

    (initiator, responder)
}
