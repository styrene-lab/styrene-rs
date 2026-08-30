use std::path::{Path, PathBuf};

use styrene_interop_runner::rns_fixtures::{load_rns_index, load_rns_vector_bytes};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

#[test]
fn shared_rns_fixture_contract_validates_authorities_and_artifacts() {
    let index = load_rns_index(&workspace_root()).expect("shared RNS fixture index must validate");

    assert!(index.authorities.contains_key("rns-1.4.2"));
    assert!(index.authorities.contains_key("rns-1.5.1"));
    for vector in &index.vectors {
        assert!(
            !load_rns_vector_bytes(&index, &vector.id)
                .expect("shared loader must verify indexed artifact")
                .is_empty()
        );
    }
}
