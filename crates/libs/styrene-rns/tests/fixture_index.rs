mod common;

use common::{load_rns_index, load_rns_vector_bytes, rns_vector};

const RNS_1_4_2: &str = "b48b96e61676504e0a4e527b33b9a0b4495c6872";
const RNS_1_5_1: &str = "149e4151095adf098b8f53eab0c03b37169e8559";
const RNS_1_5_2: &str = "ea98db4f53dcf0defc0e71a16e60d28b1229c4e6";

#[derive(serde::Deserialize)]
struct EmptyCarrierCase {
    source_symbol: String,
    empty_input: String,
    inbound_calls: u64,
    rx_bytes_delta: u64,
}

#[test]
fn committed_v2_index_exposes_immutable_authorities() {
    let index = load_rns_index().expect("committed RNS fixture index must validate");
    assert_eq!(index.schema_version, 2);
    assert_eq!(index.authorities["rns-1.4.2"].revision, RNS_1_4_2);
    assert_eq!(index.authorities["rns-1.4.2"].release, "1.4.2");
    assert_eq!(index.authorities["rns-1.5.1"].revision, RNS_1_5_1);
    assert_eq!(index.authorities["rns-1.5.1"].release, "1.5.1");
    assert_eq!(index.authorities["rns-1.5.2"].revision, RNS_1_5_2);
    assert_eq!(index.authorities["rns-1.5.2"].release, "1.5.2");
    assert!(
        index
            .authorities
            .values()
            .all(|authority| authority.repository == "https://github.com/markqvist/Reticulum.git")
    );
}

#[test]
fn every_committed_vector_resolves_to_digest_checked_bytes() {
    let index = load_rns_index().expect("committed RNS fixture index must validate");
    assert!(!index.vectors.is_empty());
    for vector in &index.vectors {
        let selected = rns_vector(&index, &vector.id).expect("indexed vector must resolve");
        assert_eq!(selected.id, vector.id);
        assert!(
            !load_rns_vector_bytes(&index, &vector.id)
                .expect("indexed artifact must pass digest validation")
                .is_empty()
        );
    }
}

#[test]
fn reticulum_1_5_2_empty_carrier_evidence_is_complete() {
    let index = load_rns_index().expect("committed RNS fixture index must validate");
    let vector =
        rns_vector(&index, "rns-1.5.2-empty-carrier-input").expect("empty-carrier vector metadata");
    let bytes = load_rns_vector_bytes(&index, "rns-1.5.2-empty-carrier-input")
        .expect("empty-carrier fixture");
    let cases: Vec<EmptyCarrierCase> = serde_json::from_slice(&bytes).expect("fixture cases");

    assert_eq!(cases.len(), 8);
    assert_eq!(
        vector.source_symbols,
        cases.iter().map(|case| case.source_symbol.clone()).collect::<Vec<_>>()
    );
    assert_eq!(
        vector.expected,
        serde_json::json!({
            "type": "acceptance-matrix",
            "empty_input": "ignored",
            "inbound_calls": 0,
            "rx_bytes_delta": 0,
        })
    );
    for case in &cases {
        assert!(case.source_symbol.ends_with(".process_incoming"));
        assert_eq!(case.empty_input, "ignored");
        assert_eq!(case.inbound_calls, 0);
        assert_eq!(case.rx_bytes_delta, 0);
    }
}

#[test]
fn unknown_vector_id_is_rejected() {
    let index = load_rns_index().expect("committed RNS fixture index must validate");
    assert!(rns_vector(&index, "missing-vector").is_err());
}

#[test]
fn cross_wave_consumers_use_the_shared_v2_loader_and_authorities() {
    let (index, consumers) = common::load_rns_fixture_consumers()
        .expect("cross-wave RNS fixture consumers must validate through the shared loader");
    assert_eq!(
        consumers.iter().map(|consumer| consumer.change_id.as_str()).collect::<Vec<_>>(),
        ["beechat-rns-corrections-wave", "freetak-rns-hardening-wave", "leviculum-rns-corpus-wave",]
    );
    for consumer in consumers {
        for authority_id in consumer.authority_ids {
            assert!(index.authorities.contains_key(&authority_id));
        }
        for vector_id in consumer.vector_ids {
            assert!(
                !load_rns_vector_bytes(&index, &vector_id).expect("consumer vector").is_empty()
            );
        }
    }
}
