#![cfg(feature = "transport")]

mod common;

use rns_core::transport::core_transport::deadlines::{link_proof_extra_grace, medium_path_grace};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct DeadlineMatrix {
    medium_path_grace: Vec<DeadlineCase>,
    link_proof_extra_grace: Vec<DeadlineCase>,
}

#[derive(Debug, Deserialize)]
struct DeadlineCase {
    bitrate: Option<u64>,
    expected_nanos: u64,
}

#[test]
fn deadline_formulas_match_pinned_reticulum_matrix() {
    let index = common::load_rns_index().expect("committed RNS fixture index");
    let bytes = common::load_rns_vector_bytes(&index, "rns-1.5.1-bitrate-deadlines")
        .expect("canonical bitrate deadline fixture");
    let matrix: DeadlineMatrix =
        serde_json::from_slice(&bytes).expect("valid bitrate deadline fixture");

    for case in matrix.medium_path_grace {
        assert_eq!(
            medium_path_grace(case.bitrate).as_nanos(),
            u128::from(case.expected_nanos),
            "medium path grace mismatch for {:?}",
            case.bitrate
        );
    }
    for case in matrix.link_proof_extra_grace {
        assert_eq!(
            link_proof_extra_grace(case.bitrate).as_nanos(),
            u128::from(case.expected_nanos),
            "link proof grace mismatch for {:?}",
            case.bitrate
        );
    }
}
