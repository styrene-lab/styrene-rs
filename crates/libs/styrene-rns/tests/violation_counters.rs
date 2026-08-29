#![cfg(feature = "transport")]

use rns_core::transport::iface::{
    InterfaceDropReason, InterfaceFilterSnapshot, InterfaceStats, InterfaceViolationSnapshot,
};

#[test]
fn every_drop_reason_changes_exactly_one_typed_counter() {
    let cases = [
        (
            InterfaceDropReason::MalformedFrame,
            InterfaceViolationSnapshot { malformed_frame: 1, ..Default::default() },
            InterfaceFilterSnapshot::default(),
        ),
        (
            InterfaceDropReason::IfacFailure,
            InterfaceViolationSnapshot { ifac_failure: 1, ..Default::default() },
            InterfaceFilterSnapshot::default(),
        ),
        (
            InterfaceDropReason::InvalidAnnounce,
            InterfaceViolationSnapshot { invalid_announce: 1, ..Default::default() },
            InterfaceFilterSnapshot::default(),
        ),
        (
            InterfaceDropReason::PreValidationLink,
            InterfaceViolationSnapshot { pre_validation_link: 1, ..Default::default() },
            InterfaceFilterSnapshot::default(),
        ),
        (
            InterfaceDropReason::ExcessivePathRequestTags,
            InterfaceViolationSnapshot { excessive_path_request_tags: 1, ..Default::default() },
            InterfaceFilterSnapshot::default(),
        ),
        (
            InterfaceDropReason::ValidBlackhole,
            InterfaceViolationSnapshot::default(),
            InterfaceFilterSnapshot { valid_blackhole: 1 },
        ),
    ];

    for (reason, violations, filters) in cases {
        let stats = InterfaceStats::new();
        stats.record_drop(reason);
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.tx_bytes, 0);
        assert_eq!(snapshot.rx_bytes, 0);
        assert_eq!(snapshot.violations, violations);
        assert_eq!(snapshot.filters, filters);
    }
}
