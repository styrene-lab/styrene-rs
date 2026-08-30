use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[test]
fn repository_signing_corpus_provenance_matches_source_tree() {
    let root = workspace_root();
    let manifest_path = root
        .join("crates/libs/styrene-identity/tests/vectors/repository-signing-v1/provenance.toml");
    let manifest: toml::Value = std::fs::read_to_string(&manifest_path)
        .expect("read repository-signing provenance")
        .parse()
        .expect("parse repository-signing provenance");

    assert_eq!(manifest["schema_version"].as_integer(), Some(1));
    assert_eq!(manifest["profile"].as_str(), Some("styrene-repository-signing-v1"));
    assert!(
        manifest["warning"]
            .as_str()
            .is_some_and(|warning| warning.contains("public test material"))
    );

    let status = manifest["status"].as_str().expect("provenance status");
    let revision = manifest["generator_revision"].as_str().expect("generator revision");
    match status {
        "candidate" | "released" => assert!(
            revision.len() == 40
                && revision
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
            "corpus provenance requires a full lowercase commit SHA"
        ),
        other => panic!("unsupported provenance status: {other}"),
    }

    for section in ["generators", "artifacts"] {
        for entry in manifest[section].as_array().expect("provenance entries") {
            let path = entry["path"].as_str().expect("entry path");
            let expected = entry["sha256"].as_str().expect("entry digest");
            let bytes = std::fs::read(root.join(path)).expect("read provenance entry");
            assert_eq!(hex::encode(Sha256::digest(bytes)), expected, "{path}");
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("styrene-identity is three levels below the workspace")
        .to_path_buf()
}
