use std::path::{Path, PathBuf};

use styrene_interop_runner::{PINNED_SCENARIOS, rns_handoffs::load_rns_live_handoff};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

#[test]
fn rns_1_5_1_live_handoffs_are_bounded_and_unregistered() {
    let root = workspace_root();
    let handoff = load_rns_live_handoff(&root).expect("valid RNS live handoff manifest");
    let product = std::fs::read_to_string(root.join("product/capabilities-v1.toml"))
        .expect("product capability registry");
    let workflow = std::fs::read_to_string(root.join(".github/workflows/live-interop.yml"))
        .expect("live interop workflow");

    for scenario in handoff.scenarios {
        assert!(PINNED_SCENARIOS.iter().all(|pinned| pinned.id.as_str() != scenario.id));
        assert!(!product.contains(&format!("id = \"{}\"", scenario.id)));
        assert!(!workflow.contains(&scenario.id));
    }
}
