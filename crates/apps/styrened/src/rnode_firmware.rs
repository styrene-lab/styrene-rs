//! Composition mapping from raw RNode transport facts to firmware policy facts.

use rns_core::transport::iface::rnode::RNodeMetadata;
use styrene_rnode_firmware::{ConfigurationState, McuFamily, Sha256Digest, TargetObservation};

const MCU_1284P: u8 = 0x91;
const MCU_2560: u8 = 0x92;
const MCU_ESP32: u8 = 0x81;
const MCU_NRF52: u8 = 0x71;

/// Preserve raw RNode observations without promoting them to an exact catalog target.
#[must_use]
pub fn target_observation(generation: u64, metadata: &RNodeMetadata) -> TargetObservation {
    let mut target = TargetObservation::new(
        match metadata.mcu {
            Some(MCU_1284P) => McuFamily::Avr1284p,
            Some(MCU_2560) => McuFamily::AvrMega2560,
            Some(MCU_ESP32) => McuFamily::Esp32,
            Some(MCU_NRF52) => McuFamily::Nrf52840,
            _ => McuFamily::Unknown,
        },
        ConfigurationState::Unknown,
    );
    target.generation = generation;
    target.platform_code = metadata.platform;
    target.mcu_code = metadata.mcu;
    target.board_code = metadata.board;
    target.product_code = metadata.product;
    target.model_code = metadata.model;
    target.hardware_revision_code = metadata.hardware_revision;
    target.firmware_version = metadata.firmware_version.map(|version| version.to_string());
    target.target_application_hash = metadata.target_firmware_hash.map(Sha256Digest::from_bytes);
    target.running_application_hash = metadata.running_firmware_hash.map(Sha256Digest::from_bytes);
    target
}

#[cfg(test)]
mod tests {
    use rns_core::transport::iface::rnode::{RNodeFirmwareVersion, RNodeMetadata};
    use styrene_rnode_firmware::{
        CapabilityDecision, CapabilityReason, CapabilityRequest, ExecutorClass, FirmwareOperation,
        HostClass, McuFamily,
    };

    use super::*;

    #[test]
    fn raw_observation_is_retained_without_enabling_an_exact_target() {
        let target = target_observation(
            9,
            &RNodeMetadata {
                firmware_version: Some(RNodeFirmwareVersion { major: 1, minor: 86 }),
                platform: Some(0x80),
                mcu: Some(0x81),
                board: Some(0x3a),
                product: Some(0x03),
                model: Some(0xa6),
                hardware_revision: Some(0x01),
                target_firmware_hash: Some([0x11; 32]),
                running_firmware_hash: Some([0x22; 32]),
            },
        );

        assert_eq!(target.generation, 9);
        assert_eq!(target.mcu_family, McuFamily::Esp32);
        assert_eq!(target.platform_code, Some(0x80));
        assert_eq!(target.mcu_code, Some(0x81));
        assert_eq!(target.board_code, Some(0x3a));
        assert_eq!(target.product_code, Some(0x03));
        assert_eq!(target.model_code, Some(0xa6));
        assert_eq!(target.hardware_revision_code, Some(0x01));
        assert_eq!(target.firmware_version.as_deref(), Some("1.86"));
        assert_eq!(
            target.target_application_hash.as_ref().map(Sha256Digest::as_str),
            Some("11".repeat(32).as_str())
        );
        assert_eq!(
            target.running_application_hash.as_ref().map(Sha256Digest::as_str),
            Some("22".repeat(32).as_str())
        );

        let result = CapabilityRequest {
            host: HostClass::Desktop,
            operation: FirmwareOperation::Upgrade,
            executor: Some(ExecutorClass::HostSerialEsp),
            target,
            physical_acceptance: true,
        }
        .evaluate();
        assert_eq!(result.decision, CapabilityDecision::Deny);
        assert_eq!(result.reason, CapabilityReason::ExactTargetUnknown);
    }

    #[test]
    fn ambiguous_esp32_code_is_not_promoted_to_esp32s3() {
        let target =
            target_observation(1, &RNodeMetadata { mcu: Some(0x81), ..Default::default() });
        assert_eq!(target.mcu_family, McuFamily::Esp32);
    }
}
