#![cfg(all(feature = "interop-tests", feature = "transport"))]

mod common;

use rns_core::buffer::OutputBuffer;
use rns_core::transport::iface::hdlc::Hdlc;

#[derive(serde::Deserialize)]
struct HdlcVector {
    description: String,
    decoded_hex: String,
    encoded_hex: String,
}

#[test]
fn hdlc_encode_matches_python() {
    let vectors: Vec<HdlcVector> = common::load_fixture("hdlc_vectors.json");
    assert!(!vectors.is_empty(), "no HDLC vectors loaded");

    for v in &vectors {
        let payload = common::hex_decode(&v.decoded_hex);
        let expected_encoded = common::hex_decode(&v.encoded_hex);

        let mut buf = vec![0u8; payload.len() * 2 + 16];
        let mut output = OutputBuffer::new(&mut buf);
        Hdlc::encode(&payload, &mut output)
            .unwrap_or_else(|e| panic!("{}: encode failed: {e:?}", v.description));

        let encoded = output.as_slice();
        assert_eq!(
            hex::encode(encoded),
            v.encoded_hex,
            "{}: encoded output mismatch (got {} bytes, expected {} bytes)",
            v.description,
            encoded.len(),
            expected_encoded.len(),
        );
    }
}

#[test]
fn hdlc_decode_matches_python() {
    let vectors: Vec<HdlcVector> = common::load_fixture("hdlc_vectors.json");

    for v in &vectors {
        let encoded = common::hex_decode(&v.encoded_hex);
        let expected_payload = common::hex_decode(&v.decoded_hex);

        let mut buf = vec![0u8; encoded.len()];
        let mut output = OutputBuffer::new(&mut buf);
        Hdlc::decode(&encoded, &mut output)
            .unwrap_or_else(|e| panic!("{}: decode failed: {e:?}", v.description));

        let decoded = output.as_slice();
        assert_eq!(
            hex::encode(decoded),
            v.decoded_hex,
            "{}: decoded output mismatch",
            v.description,
        );

        assert_eq!(
            decoded,
            expected_payload.as_slice(),
            "{}: decoded bytes mismatch",
            v.description,
        );
    }
}

#[test]
fn hdlc_roundtrip() {
    let vectors: Vec<HdlcVector> = common::load_fixture("hdlc_vectors.json");

    for v in &vectors {
        let payload = common::hex_decode(&v.decoded_hex);

        // Encode
        let mut enc_buf = vec![0u8; payload.len() * 2 + 16];
        let mut enc_output = OutputBuffer::new(&mut enc_buf);
        Hdlc::encode(&payload, &mut enc_output)
            .unwrap_or_else(|e| panic!("{}: encode failed: {e:?}", v.description));
        let encoded = enc_output.as_slice().to_vec();

        // Decode back
        let mut dec_buf = vec![0u8; encoded.len()];
        let mut dec_output = OutputBuffer::new(&mut dec_buf);
        Hdlc::decode(&encoded, &mut dec_output)
            .unwrap_or_else(|e| panic!("{}: decode failed: {e:?}", v.description));
        let decoded = dec_output.as_slice();

        assert_eq!(
            decoded,
            payload.as_slice(),
            "{}: encode→decode roundtrip failed",
            v.description,
        );
    }
}

#[test]
fn hdlc_find_frame() {
    let vectors: Vec<HdlcVector> = common::load_fixture("hdlc_vectors.json");

    for v in &vectors {
        let encoded = common::hex_decode(&v.encoded_hex);

        let result = Hdlc::find(&encoded);

        // All our test vectors are valid HDLC frames with start and end flags
        let (start, end) = result
            .unwrap_or_else(|| panic!("{}: find() returned None for valid frame", v.description));

        assert_eq!(start, 0, "{}: frame should start at index 0", v.description);
        assert_eq!(end, encoded.len() - 1, "{}: frame should end at last byte", v.description);
    }
}
