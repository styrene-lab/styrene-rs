use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use styrene_interop_runner::rns_fixtures::{load_rns_index, load_rns_vector_bytes};
use tempfile::TempDir;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn fixture_index(artifact: &str, sha256: &str) -> Value {
    json!({
        "schema_version": 2,
        "authorities": {
            "rns-test": {
                "repository": "https://example.invalid/Reticulum.git",
                "revision": "0123456789abcdef0123456789abcdef01234567",
                "release": "test"
            }
        },
        "vectors": [{
            "id": "packet",
            "authority_id": "rns-test",
            "kind": "packet",
            "artifact": artifact,
            "sha256": sha256,
            "generator": "manual-copy",
            "source_symbols": ["RNS.Packet"],
            "expected": {"type": "accepted"}
        }]
    })
}

fn write_index(root: &Path, index: &Value) -> PathBuf {
    let path = root.join("index.json");
    fs::write(&path, serde_json::to_vec(index).expect("fixture index must serialize"))
        .expect("fixture index must be writable");
    path
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

#[test]
fn shared_loader_rejects_malformed_contract_and_traversal() {
    let temp = TempDir::new().expect("temporary fixture root");
    let mut index = fixture_index("../outside.bin", "00");
    index["schema_version"] = json!(1);
    index["authorities"]["rns-test"]["revision"] = json!("mutable");
    index["vectors"][0]["authority_id"] = json!("unknown");
    index["vectors"][0]["source_symbols"] = json!([]);
    index["vectors"][0]["expected"] = json!({});
    let duplicate = index["vectors"][0].clone();
    index["vectors"].as_array_mut().expect("vectors must be an array").push(duplicate);
    let path = write_index(temp.path(), &index);

    let errors = styrene_interop_runner::rns_fixtures::load_rns_index_from(temp.path(), &path)
        .expect_err("malformed fixture index must fail");
    for expected in [
        "unsupported schema version",
        "revision must be a lowercase full commit SHA",
        "missing or duplicate vector id",
        "unknown authority",
        "artifact must be repository-relative",
        "source symbols must not be empty",
        "expected outcome must have a type",
    ] {
        assert!(
            errors.iter().any(|error| error.contains(expected)),
            "missing {expected}: {errors:?}"
        );
    }
}

#[test]
fn vector_reload_rechecks_digest_after_index_validation() {
    let temp = TempDir::new().expect("temporary fixture root");
    let artifact = temp.path().join("packet.bin");
    fs::write(&artifact, b"original").expect("artifact must be writable");
    let index = fixture_index("packet.bin", &hex::encode(Sha256::digest(b"original")));
    let path = write_index(temp.path(), &index);
    let loaded = styrene_interop_runner::rns_fixtures::load_rns_index_from(temp.path(), &path)
        .expect("valid fixture index");

    fs::write(artifact, b"changed").expect("artifact must be replaceable");
    assert!(load_rns_vector_bytes(&loaded, "packet").is_err());
}

#[cfg(unix)]
#[test]
fn shared_loader_rejects_artifact_symlink_outside_root() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().expect("temporary directory");
    let root = temp.path().join("root");
    fs::create_dir(&root).expect("fixture root must be creatable");
    let outside = temp.path().join("outside.bin");
    fs::write(&outside, b"outside").expect("outside artifact must be writable");
    symlink(&outside, root.join("packet.bin")).expect("artifact symlink must be creatable");
    let index = fixture_index("packet.bin", &hex::encode(Sha256::digest(b"outside")));
    let path = write_index(&root, &index);

    let errors = styrene_interop_runner::rns_fixtures::load_rns_index_from(&root, &path)
        .expect_err("artifact symlink escape must fail");
    assert!(errors.iter().any(|error| error.contains("not a file within the repository root")));
}
