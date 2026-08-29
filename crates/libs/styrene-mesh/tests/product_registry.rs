use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("canonical repository root")
}

fn document(path: &str) -> toml::Value {
    std::fs::read_to_string(root().join(path))
        .unwrap_or_else(|error| panic!("read {path}: {error}"))
        .parse()
        .unwrap_or_else(|error| panic!("parse {path}: {error}"))
}

fn tables<'a>(value: &'a toml::Value, key: &str) -> &'a Vec<toml::Value> {
    value.get(key).and_then(toml::Value::as_array).unwrap_or_else(|| panic!("missing {key}"))
}

fn id_set(entries: &[toml::Value], section: &str) -> HashSet<String> {
    let mut ids = HashSet::new();
    for entry in entries {
        let id = entry
            .get("id")
            .and_then(toml::Value::as_str)
            .unwrap_or_else(|| panic!("{section} entry lacks id"));
        assert!(ids.insert(id.to_string()), "duplicate {section} id {id}");
    }
    assert!(!ids.is_empty(), "{section} must not be empty");
    ids
}

fn parity_promotion_errors(registry: &toml::Value) -> Vec<String> {
    let mut errors = Vec::new();
    let gates = tables(registry, "parity_gates")
        .iter()
        .filter_map(|gate| gate.get("id").and_then(toml::Value::as_str).map(|id| (id, gate)))
        .collect::<HashMap<_, _>>();
    for claim in tables(registry, "parity_claims") {
        let id = claim.get("id").and_then(toml::Value::as_str).unwrap_or("<missing>");
        let level = claim.get("level").and_then(toml::Value::as_str).unwrap_or("<missing>");
        if !matches!(level, "unsupported" | "experimental" | "verified" | "degraded") {
            errors.push(format!("{id}: invalid parity level {level}"));
            continue;
        }
        if level != "verified" {
            continue;
        }
        let required = claim
            .get("required_gates")
            .and_then(toml::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if required.is_empty() {
            errors.push(format!("{id}: verified claim requires at least one gate"));
        }
        for gate_id in required.iter().filter_map(toml::Value::as_str) {
            let Some(gate) = gates.get(gate_id) else {
                errors.push(format!("{id}: unknown required gate {gate_id}"));
                continue;
            };
            for (key, expected) in [("automated", true), ("enabled", true), ("ignored", false)] {
                if gate.get(key).and_then(toml::Value::as_bool) != Some(expected) {
                    errors
                        .push(format!("{id}: required gate {gate_id} must have {key}={expected}"));
                }
            }
            if gate.get("protocol").and_then(toml::Value::as_str) != Some("native") {
                errors
                    .push(format!("{id}: required gate {gate_id} is not native protocol evidence"));
            }
            if gate.get("upstreams").and_then(toml::Value::as_array).is_none_or(Vec::is_empty) {
                errors.push(format!("{id}: required gate {gate_id} has no pinned upstream"));
            }
            if gate.get("command").and_then(toml::Value::as_str).is_none_or(str::is_empty) {
                errors.push(format!("{id}: required gate {gate_id} has no command"));
            }
        }
    }
    errors
}

#[test]
fn product_registry_references_existing_evidence_and_valid_gates() {
    let registry = document("product/capabilities-v1.toml");
    let capabilities = tables(&registry, "capabilities");
    let capability_ids = id_set(capabilities, "capability");
    let gates = tables(&registry, "parity_gates");
    let gate_ids = id_set(gates, "parity gate");

    for entry in capabilities.iter().chain(gates.iter()) {
        if let Some(evidence) = entry.get("evidence").and_then(toml::Value::as_array) {
            for path in evidence {
                let path = path.as_str().expect("evidence path is a string");
                assert!(root().join(path).exists(), "missing registry evidence {path}");
            }
        }
    }
    for claim in tables(&registry, "parity_claims") {
        for key in ["required_gates", "evidence_gates"] {
            if let Some(gates) = claim.get(key).and_then(toml::Value::as_array) {
                for gate in gates {
                    assert!(
                        gate_ids.contains(gate.as_str().unwrap()),
                        "claim references unknown gate"
                    );
                }
            }
        }
    }
    assert!(
        parity_promotion_errors(&registry).is_empty(),
        "invalid parity promotion: {:?}",
        parity_promotion_errors(&registry)
    );

    for manifest in [
        "product/manifests/constrained-communicator.toml",
        "product/manifests/full-workstation.toml",
    ] {
        let manifest = document(manifest);
        for key in ["required", "experimental", "planned", "excluded"] {
            if let Some(ids) = manifest.get(key).and_then(toml::Value::as_array) {
                for id in ids {
                    let id = id.as_str().unwrap();
                    assert!(
                        capability_ids.contains(id),
                        "product manifest references unknown capability {id}"
                    );
                }
            }
        }
    }
}

fn promotion_fixture(level: &str, required: &str) -> toml::Value {
    format!(
        r#"
[[parity_gates]]
id = "native"
kind = "fixture"
automated = true
enabled = true
ignored = false
protocol = "native"
upstreams = ["pinned"]
command = "cargo test"

[[parity_claims]]
id = "claim"
level = "{level}"
required_gates = {required}
"#
    )
    .parse()
    .expect("parse promotion fixture")
}

#[test]
fn verified_promotion_requires_runnable_required_gates() {
    assert!(parity_promotion_errors(&promotion_fixture("verified", "[\"native\"]")).is_empty());
    for (field, expected) in
        [("enabled", "enabled=true"), ("ignored", "ignored=false"), ("automated", "automated=true")]
    {
        let mut fixture = promotion_fixture("verified", "[\"native\"]");
        fixture["parity_gates"][0][field] = toml::Value::Boolean(field == "ignored");
        let errors = parity_promotion_errors(&fixture);
        assert!(errors.iter().any(|error| error.contains(expected)), "{errors:?}");
    }
    let errors = parity_promotion_errors(&promotion_fixture("verified", "[]"));
    assert!(errors.iter().any(|error| error.contains("at least one gate")), "{errors:?}");
}

#[test]
fn unreachable_supported_level_is_rejected() {
    let errors = parity_promotion_errors(&promotion_fixture("supported", "[\"native\"]"));
    assert!(errors.iter().any(|error| error.contains("invalid parity level")), "{errors:?}");
}

#[test]
fn committed_fixture_provenance_digests_match() {
    let provenance = document("tests/interop/fixtures/provenance-v1.toml");
    let upstream_ids = id_set(tables(&provenance, "upstreams"), "fixture upstream");
    for upstream in tables(&provenance, "upstreams") {
        let revision = upstream.get("revision").and_then(toml::Value::as_str).unwrap();
        assert_eq!(revision.len(), 40);
        assert!(revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    for fixture_set in tables(&provenance, "fixture_sets") {
        let upstream = fixture_set.get("reference_upstream").and_then(toml::Value::as_str).unwrap();
        assert!(upstream_ids.contains(upstream));
        let artifacts = fixture_set.get("artifacts").and_then(toml::Value::as_array).unwrap();
        assert!(!artifacts.is_empty());
        for artifact in artifacts {
            let path = artifact.get("path").and_then(toml::Value::as_str).unwrap();
            let expected = artifact.get("sha256").and_then(toml::Value::as_str).unwrap();
            let actual = hex::encode(Sha256::digest(std::fs::read(root().join(path)).unwrap()));
            assert_eq!(actual, expected, "fixture digest mismatch for {path}");
        }
    }
}

#[test]
fn legacy_rns_fixture_identity_paths_and_digests_are_unchanged() {
    let provenance = document("tests/interop/fixtures/provenance-v1.toml");
    assert_eq!(provenance.get("schema_version").and_then(toml::Value::as_integer), Some(1));
    let python_rns = tables(&provenance, "upstreams")
        .iter()
        .find(|entry| entry.get("id").and_then(toml::Value::as_str) == Some("python-rns"))
        .expect("python-rns authority");
    assert_eq!(
        python_rns.get("revision").and_then(toml::Value::as_str),
        Some("b48b96e61676504e0a4e527b33b9a0b4495c6872")
    );
    assert_eq!(python_rns.get("version").and_then(toml::Value::as_str), Some("1.4.2"));

    let expected = [
        (
            "rns-python-vectors",
            "tests/interop/fixtures/identity_vectors.json",
            "7268459b46bbae5680b159e1b751f5928a88ff3a6c575dcb3277a53680993e8d",
        ),
        (
            "rns-python-vectors",
            "tests/interop/fixtures/packet_vectors.json",
            "470c63e49d64e7f5474c6550860f40ba4c464119f4a73554a835b0c9b2b6048a",
        ),
        (
            "rns-python-vectors",
            "tests/interop/fixtures/announce_vectors.json",
            "480dbbbfa11ac35b29440f9bf11d2950ccd13fd5fdfc5e08597bca004550bea8",
        ),
        (
            "rns-python-vectors",
            "tests/interop/fixtures/fernet_vectors.json",
            "3a24c1598da9cb9b5b8ce28d7dd1aff65d94d07918c9e5fbafd29456d007df38",
        ),
        (
            "rns-python-vectors",
            "tests/interop/fixtures/hdlc_vectors.json",
            "a73c56a22370043499719122db8c418acb6559c4d6acfb886281fad3fc139e4a",
        ),
        (
            "rns-ifac-vectors",
            "tests/interop/fixtures/ifac_vectors.json",
            "c255dbb52f575094c77551c94066fbab27db806f1e57a8273d3a1dd019d37439",
        ),
    ];
    let fixture_sets = tables(&provenance, "fixture_sets");
    for (fixture_id, path, digest) in expected {
        let fixture_set = fixture_sets
            .iter()
            .find(|entry| entry.get("id").and_then(toml::Value::as_str) == Some(fixture_id))
            .unwrap_or_else(|| panic!("missing legacy fixture set {fixture_id}"));
        let artifact = fixture_set
            .get("artifacts")
            .and_then(toml::Value::as_array)
            .and_then(|artifacts| {
                artifacts
                    .iter()
                    .find(|entry| entry.get("path").and_then(toml::Value::as_str) == Some(path))
            })
            .unwrap_or_else(|| panic!("missing legacy fixture artifact {path}"));
        assert_eq!(artifact.get("sha256").and_then(toml::Value::as_str), Some(digest));
        assert_eq!(hex::encode(Sha256::digest(std::fs::read(root().join(path)).unwrap())), digest);
    }
}
