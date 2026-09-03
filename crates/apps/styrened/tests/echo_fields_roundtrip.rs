//! An echo response's correlation fields must survive the inbound projection.
use rns_core::identity::PrivateIdentity;
use styrened::inbound_delivery::{InboundPayloadMode, decode_inbound_payload};
use styrened::lxmf_bridge::build_wire_message;

#[test]
fn echo_fields_survive_inbound_projection() {
    let signer = PrivateIdentity::new_from_name("echo-fields-signer");
    let mut source = [0u8; 16];
    source.copy_from_slice(signer.address_hash().as_slice());
    let destination = [7u8; 16];
    let fields = serde_json::json!({
        "styrene_echo": { "request_id": "a".repeat(64), "response": true }
    });
    let wire = build_wire_message(
        source,
        destination,
        "[auto-reply]",
        "echo body",
        Some(fields.clone()),
        &signer,
    )
    .expect("wire message");
    let record = decode_inbound_payload(destination, &wire, InboundPayloadMode::FullWire)
        .expect("decoded inbound projection");
    assert_eq!(record.title, "[auto-reply]");
    assert_eq!(record.fields, Some(fields), "projection must carry the decoded LXMF fields");
}
