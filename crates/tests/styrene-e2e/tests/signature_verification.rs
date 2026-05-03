//! LXMF signature verification scenarios.
//!
//! Tests that the inbound worker verifies Ed25519 signatures on LXMF
//! messages. Legitimate signed messages should deliver normally.
//! The signature is the root of the identity trust chain.

use std::time::Duration;

use styrene_e2e::helpers::{
    await_inbound_count, await_inbound_message, two_connected_nodes, with_timeout,
};

#[tokio::test]
async fn legitimate_signed_message_delivers() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-sig", "bob-sig").await;

        // Normal send_chat produces a signed LXMF message.
        // With signature verification wired in, this should still deliver.
        alice.send_chat(&bob.delivery_hash, "signed message").await.expect("send");

        let received = await_inbound_message(&bob.app_context, Duration::from_secs(15)).await;
        assert_eq!(received.content, "signed message");
        assert_eq!(received.source, alice.identity_hash, "source attribution should match sender");
    })
    .await;
}

#[tokio::test]
async fn bidirectional_signed_messages_deliver() {
    with_timeout(async {
        let (alice, bob) = two_connected_nodes("alice-sig2", "bob-sig2").await;

        alice.send_chat(&bob.delivery_hash, "from alice").await.expect("a→b");
        await_inbound_count(&bob.app_context, 1, Duration::from_secs(15)).await;

        bob.send_chat(&alice.delivery_hash, "from bob").await.expect("b→a");
        await_inbound_count(&alice.app_context, 1, Duration::from_secs(15)).await;

        // Both sides verify signatures and accept
        {
            let store = bob.app_context.store().lock().expect("lock");
            let msgs = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = msgs.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 1);
            assert_eq!(inbound[0].content, "from alice");
            assert_eq!(inbound[0].source, alice.identity_hash);
        }

        {
            let store = alice.app_context.store().lock().expect("lock");
            let msgs = store.list_messages(100, None).expect("list");
            let inbound: Vec<_> = msgs.iter().filter(|m| m.direction == "in").collect();
            assert_eq!(inbound.len(), 1);
            assert_eq!(inbound[0].content, "from bob");
            assert_eq!(inbound[0].source, bob.identity_hash);
        }
    })
    .await;
}

#[tokio::test]
async fn verify_inbound_signature_function_works() {
    // Unit-level test: build a signed wire message, verify it passes,
    // then tamper with the signature and verify it fails.
    use rns_core::identity::PrivateIdentity;
    use styrened::inbound_delivery::InboundPayloadMode;

    let sender = PrivateIdentity::new_from_name("sig-sender");
    let receiver = PrivateIdentity::new_from_name("sig-receiver");

    let mut sender_hash = [0u8; 16];
    sender_hash.copy_from_slice(sender.address_hash().as_slice());
    let mut receiver_hash = [0u8; 16];
    receiver_hash.copy_from_slice(receiver.address_hash().as_slice());

    // Build and sign a legitimate message
    let payload = styrened::lxmf_bridge::build_wire_message(
        sender_hash,
        receiver_hash,
        "test",
        "hello",
        None,
        &sender,
    )
    .expect("build");

    // Verify with correct identity should pass
    let result = styrened::inbound_delivery::verify_inbound_signature(
        &payload,
        InboundPayloadMode::FullWire,
        receiver_hash,
        sender.as_identity(),
    );
    assert_eq!(result, Some(true), "legitimate signature should verify");

    // Verify with wrong identity should fail
    let wrong_identity = PrivateIdentity::new_from_name("wrong-identity");
    let result = styrened::inbound_delivery::verify_inbound_signature(
        &payload,
        InboundPayloadMode::FullWire,
        receiver_hash,
        wrong_identity.as_identity(),
    );
    assert_eq!(result, Some(false), "wrong identity should fail verification");

    // Tamper with signature bytes — should fail
    let mut tampered = payload.clone();
    if tampered.len() > 40 {
        tampered[35] ^= 0xFF; // flip a byte in the signature region
    }
    let result = styrened::inbound_delivery::verify_inbound_signature(
        &tampered,
        InboundPayloadMode::FullWire,
        receiver_hash,
        sender.as_identity(),
    );
    assert_eq!(result, Some(false), "tampered signature should fail verification");
}
