#![cfg(feature = "repository-signing")]

use std::path::PathBuf;

use styrene_identity::{verify_repository_signer_binding, RepositorySignerBindingErrorClass};

#[test]
fn committed_negative_corpus_has_stable_rejection_classes() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/vectors/repository-signing-v1/negative.json");
    let bytes = std::fs::read(path).expect("read committed negative corpus without modifying it");
    let corpus: serde_json::Value =
        serde_json::from_slice(&bytes).expect("committed negative corpus JSON");

    for vector in corpus["vectors"].as_array().expect("vector array") {
        let name = vector["name"].as_str().expect("vector name");
        let input = hex::decode(hex_field(&vector["input_hex"])).expect("valid input hex");
        let expected =
            error_class(vector["expected_error_class"].as_str().expect("expected error class"));
        let actual = verify_repository_signer_binding(&input)
            .map(|_| panic!("{name} unexpectedly verified"))
            .expect_err("negative vector must be rejected")
            .class();
        assert_eq!(actual, expected, "{name}");
    }
}

fn hex_field(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(hex) => hex.clone(),
        serde_json::Value::Array(chunks) => {
            chunks.iter().map(|chunk| chunk.as_str().expect("hex chunk")).collect()
        }
        _ => panic!("input_hex must be a string or string array"),
    }
}

fn error_class(name: &str) -> RepositorySignerBindingErrorClass {
    match name {
        "Format" => RepositorySignerBindingErrorClass::Format,
        "TooLarge" => RepositorySignerBindingErrorClass::TooLarge,
        "Canonical" => RepositorySignerBindingErrorClass::Canonical,
        "Semantic" => RepositorySignerBindingErrorClass::Semantic,
        "IdentityMismatch" => RepositorySignerBindingErrorClass::IdentityMismatch,
        "Signature" => RepositorySignerBindingErrorClass::Signature,
        other => panic!("unknown RepositorySignerBindingErrorClass: {other}"),
    }
}
