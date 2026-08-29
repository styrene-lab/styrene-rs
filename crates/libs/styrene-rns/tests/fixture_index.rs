mod common;

use common::{load_rns_index, load_rns_vector_bytes, rns_vector};

const RNS_1_4_2: &str = "b48b96e61676504e0a4e527b33b9a0b4495c6872";
const RNS_1_5_1: &str = "149e4151095adf098b8f53eab0c03b37169e8559";

#[test]
fn committed_v2_index_exposes_both_immutable_authorities() {
    let index = load_rns_index().expect("committed RNS fixture index must validate");
    assert_eq!(index.schema_version, 2);
    assert_eq!(index.authorities["rns-1.4.2"].revision, RNS_1_4_2);
    assert_eq!(index.authorities["rns-1.4.2"].release, "1.4.2");
    assert_eq!(index.authorities["rns-1.5.1"].revision, RNS_1_5_1);
    assert_eq!(index.authorities["rns-1.5.1"].release, "1.5.1");
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
fn unknown_vector_id_is_rejected() {
    let index = load_rns_index().expect("committed RNS fixture index must validate");
    assert!(rns_vector(&index, "missing-vector").is_err());
}
