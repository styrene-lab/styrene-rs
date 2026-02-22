use super::{native, BleRuntimeSettings};
use reticulum_daemon::config::InterfaceConfig;

pub(super) async fn startup(
    iface: &InterfaceConfig,
    settings: &BleRuntimeSettings,
) -> Result<(), String> {
    native::startup_with_backend("windows", iface, settings).await
}
