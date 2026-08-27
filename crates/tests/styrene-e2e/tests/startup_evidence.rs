use styrene_e2e::node::TestNodeBuilder;
use styrened::startup_contract::{capabilities, components, EvidenceScope, RuntimeKind};

#[tokio::test]
async fn test_only_page_handler_is_internal_styrene_evidence() {
    let node = TestNodeBuilder::new("internal-page-evidence").build().await;
    let contract = &node.startup_contract;

    assert_eq!(contract.runtime(), RuntimeKind::E2eTest);
    assert_eq!(contract.evidence_scope(), EvidenceScope::InternalTest);
    assert!(contract.has_component(components::STYRENE_PAGE_REQUEST_HANDLER));
    assert!(contract.advertises(capabilities::STYRENE_PAGE_HOST.id()));
    assert!(!contract.can_support_production_claim(capabilities::STYRENE_PAGE_HOST.id()));
    assert!(!contract.advertises(capabilities::NATIVE_NOMADNET_HOST.id()));
    assert!(!contract.can_support_production_claim(capabilities::NATIVE_NOMADNET_HOST.id()));
    assert!(!contract.has_component(components::STYRENE_PROPAGATION_SERVICE));
    assert!(!contract.advertises(capabilities::STYRENE_PROPAGATION_HOST.id()));
}

#[tokio::test]
async fn test_only_propagation_handler_is_not_standard_lxmf_evidence() {
    let node =
        TestNodeBuilder::new("internal-propagation-evidence").propagation(true).build().await;
    let contract = &node.startup_contract;

    assert_eq!(contract.evidence_scope(), EvidenceScope::InternalTest);
    assert!(contract.has_component(components::STYRENE_PROPAGATION_SERVICE));
    assert!(contract.has_component(components::STYRENE_PROPAGATION_REQUEST_HANDLER));
    assert!(contract.advertises(capabilities::STYRENE_PROPAGATION_HOST.id()));
    assert!(!contract.can_support_production_claim(capabilities::STYRENE_PROPAGATION_HOST.id()));
    assert!(!contract.advertises(capabilities::STANDARD_LXMF_PROPAGATION.id()));
    assert!(!contract.can_support_production_claim(capabilities::STANDARD_LXMF_PROPAGATION.id()));
}
