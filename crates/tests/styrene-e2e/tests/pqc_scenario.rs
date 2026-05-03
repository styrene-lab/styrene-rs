//! PQC integrated scenario — full lifecycle as a single narrative.
//!
//! Two peers: establish PQC session, exchange multiple encrypted messages
//! in both directions, verify content, rekey the session, exchange more
//! messages with the new key, then close gracefully.

use std::net::IpAddr;

use styrene_e2e::tunnel_mock::MockTunnelBackend;
use styrene_tunnel::session::{CloseAction, PqcSession, SessionState};
use styrene_tunnel::traits::{TunnelBackend, TunnelParams};

/// Full PQC lifecycle: handshake → encrypt → decrypt → rekey-equivalent →
/// new session → encrypt again → close.
#[tokio::test]
async fn pqc_full_lifecycle_scenario() {
    let alice_hash: [u8; 16] = [0xAA; 16];
    let bob_hash: [u8; 16] = [0xBB; 16];

    // ── Phase 1: Handshake ─────────────────────────────────────────
    let mut alice = PqcSession::new(alice_hash);
    let mut bob = PqcSession::new(bob_hash);

    let initiate = alice.initiate().expect("alice initiates");
    let respond = bob.process_initiate(&initiate).expect("bob responds");
    let confirm = alice.process_respond(&respond).expect("alice confirms");
    bob.process_confirm(&confirm).expect("bob completes handshake");

    assert_eq!(alice.state(), SessionState::Established);
    assert_eq!(bob.state(), SessionState::Established);

    let session_key_1 = *alice.session_key().expect("session key");
    assert_eq!(
        &session_key_1,
        bob.session_key().expect("bob key"),
        "both sides must derive identical key"
    );

    // ── Phase 2: Tunnel establishment via mock backend ─────────────
    let mock_wg = MockTunnelBackend::new("wireguard");
    let tunnel_id = mock_wg
        .establish(TunnelParams {
            peer_identity: hex::encode(bob_hash),
            remote_endpoint: Some("10.0.0.2".parse::<IpAddr>().expect("parse")),
            remote_port: Some(51820),
            psk: session_key_1,
            peer_x25519_public: None,
            peer_mesh_ip: None,
            mtu: Some(1420),
        })
        .await
        .expect("establish tunnel");

    assert_eq!(mock_wg.establish_count(), 1);
    let info = mock_wg.status(&tunnel_id).await.expect("status");
    assert_eq!(info.peer_identity, hex::encode(bob_hash));

    // ── Phase 3: Multi-message encrypted exchange ──────────────────
    // Alice sends 5 messages to Bob
    let mut bob_received = Vec::new();
    for i in 0..5 {
        let plaintext = format!("alice-msg-{}", i);
        let encrypted = alice.encrypt_data(plaintext.as_bytes()).expect("encrypt");
        let decrypted = bob.decrypt_data(&encrypted).expect("decrypt");
        assert_eq!(decrypted, plaintext.as_bytes());
        bob_received.push(String::from_utf8(decrypted).expect("utf8"));
    }
    assert_eq!(
        bob_received,
        vec!["alice-msg-0", "alice-msg-1", "alice-msg-2", "alice-msg-3", "alice-msg-4"]
    );

    // Bob sends 3 replies
    let mut alice_received = Vec::new();
    for i in 0..3 {
        let plaintext = format!("bob-reply-{}", i);
        let encrypted = bob.encrypt_data(plaintext.as_bytes()).expect("encrypt");
        let decrypted = alice.decrypt_data(&encrypted).expect("decrypt");
        assert_eq!(decrypted, plaintext.as_bytes());
        alice_received.push(String::from_utf8(decrypted).expect("utf8"));
    }
    assert_eq!(alice_received, vec!["bob-reply-0", "bob-reply-1", "bob-reply-2"]);

    // ── Phase 4: Out-of-order delivery ─────────────────────────────
    // Alice encrypts 3 more, Bob receives them out of order
    let pkt_a = alice.encrypt_data(b"order-a").expect("enc a");
    let pkt_b = alice.encrypt_data(b"order-b").expect("enc b");
    let pkt_c = alice.encrypt_data(b"order-c").expect("enc c");

    // Deliver c, a, b (mesh-realistic scramble)
    assert_eq!(bob.decrypt_data(&pkt_c).expect("dec c"), b"order-c");
    assert_eq!(bob.decrypt_data(&pkt_a).expect("dec a"), b"order-a");
    assert_eq!(bob.decrypt_data(&pkt_b).expect("dec b"), b"order-b");

    // Replay of pkt_a must fail
    assert!(bob.decrypt_data(&pkt_a).is_err(), "replay must be rejected");

    // ── Phase 5: Rekey simulation ──────────────────────────────────
    // In practice, rekeying creates a new session. Simulate by establishing
    // a new session pair and rekeying the tunnel backend.
    let mut alice2 = PqcSession::new(alice_hash);
    let mut bob2 = PqcSession::new(bob_hash);

    let init2 = alice2.initiate().expect("rekey initiate");
    let resp2 = bob2.process_initiate(&init2).expect("rekey respond");
    let conf2 = alice2.process_respond(&resp2).expect("rekey confirm");
    bob2.process_confirm(&conf2).expect("rekey complete");

    let session_key_2 = *alice2.session_key().expect("new session key");
    assert_ne!(session_key_1, session_key_2, "rekeyed session must produce different key material");

    // Rekey the tunnel
    mock_wg.rekey(&tunnel_id, &session_key_2).await.expect("rekey tunnel");
    assert_eq!(mock_wg.rekey_log.lock().expect("lock").len(), 1);

    // Continue exchanging data with new session
    let encrypted = alice2.encrypt_data(b"post-rekey").expect("encrypt");
    let decrypted = bob2.decrypt_data(&encrypted).expect("decrypt");
    assert_eq!(decrypted, b"post-rekey");

    // Old session can't decrypt new session's data
    assert!(
        bob.decrypt_data(&encrypted).is_err(),
        "old session must not decrypt new session's data"
    );

    // ── Phase 6: Authenticated close ───────────────────────────────
    let close_action = alice2.close(0x00, Some("session complete".into())).expect("close");
    assert_eq!(alice2.state(), SessionState::Closed);

    match close_action {
        CloseAction::Authenticated(data) => {
            let plaintext = bob2.decrypt_data(&data).expect("decrypt close");
            let (reason, msg) = bob2.try_authenticated_close(&plaintext).expect("interpret close");
            assert_eq!(reason, 0x00);
            assert_eq!(msg.as_deref(), Some("session complete"));
            assert_eq!(bob2.state(), SessionState::Closed);
        }
        CloseAction::Unauthenticated(_) => {
            panic!("established session must use authenticated close");
        }
    }

    // ── Phase 7: Teardown tunnel ───────────────────────────────────
    mock_wg.teardown(&tunnel_id).await.expect("teardown");
    let remaining = mock_wg.list_tunnels().await.expect("list");
    assert!(remaining.is_empty(), "no tunnels should remain after teardown");

    // Verify the complete audit trail
    assert_eq!(mock_wg.establish_count(), 1, "exactly one tunnel established");
    assert_eq!(mock_wg.rekey_log.lock().expect("lock").len(), 1, "exactly one rekey performed");
}

/// Cross-session isolation: data from one session cannot be decrypted by another,
/// even between the same identity pair.
#[test]
fn session_isolation_between_same_peers() {
    let alice_hash: [u8; 16] = [0xAA; 16];
    let bob_hash: [u8; 16] = [0xBB; 16];

    // Session 1
    let (mut alice1, mut bob1) = establish(alice_hash, bob_hash);
    // Session 2 (same identities, different ephemeral keys)
    let (mut alice2, mut bob2) = establish(alice_hash, bob_hash);

    // Keys must differ
    assert_ne!(
        alice1.session_key().expect("key1"),
        alice2.session_key().expect("key2"),
        "parallel sessions between same peers must derive different keys"
    );

    // Data encrypted in session 1 must not decrypt in session 2
    let data1 = alice1.encrypt_data(b"session-1-data").expect("encrypt");
    assert!(bob2.decrypt_data(&data1).is_err(), "session 2 must not decrypt session 1 data");

    // And vice versa
    let data2 = alice2.encrypt_data(b"session-2-data").expect("encrypt");
    assert!(bob1.decrypt_data(&data2).is_err(), "session 1 must not decrypt session 2 data");

    // Each session works with its own data
    assert_eq!(bob1.decrypt_data(&data1).expect("decrypt in own session"), b"session-1-data");
    assert_eq!(bob2.decrypt_data(&data2).expect("decrypt in own session"), b"session-2-data");
}

fn establish(alice_hash: [u8; 16], bob_hash: [u8; 16]) -> (PqcSession, PqcSession) {
    let mut alice = PqcSession::new(alice_hash);
    let mut bob = PqcSession::new(bob_hash);
    let init = alice.initiate().expect("initiate");
    let resp = bob.process_initiate(&init).expect("respond");
    let conf = alice.process_respond(&resp).expect("confirm");
    bob.process_confirm(&conf).expect("complete");
    (alice, bob)
}
