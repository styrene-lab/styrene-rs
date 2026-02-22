use super::{
    run_startup_lifecycle, synthetic_probe_enabled, BleBackend, BleBackendError,
    BleLifecycleReport, BleRuntimeSettings,
};
use reticulum_daemon::config::InterfaceConfig;

#[derive(Default)]
struct LinuxBleBackend {
    probe_payload: Vec<u8>,
}

impl BleBackend for LinuxBleBackend {
    fn backend_name(&self) -> &'static str {
        "linux"
    }

    fn scan(&mut self, settings: &BleRuntimeSettings) -> Result<(), BleBackendError> {
        if settings
            .adapter
            .as_deref()
            .is_some_and(|adapter| adapter.eq_ignore_ascii_case("disabled"))
        {
            return Err(BleBackendError::terminal(
                "linux BLE adapter is disabled by configuration",
            ));
        }
        Ok(())
    }

    fn connect(&mut self, settings: &BleRuntimeSettings) -> Result<(), BleBackendError> {
        if settings.peripheral_id == "00:00:00:00:00:00" {
            return Err(BleBackendError::terminal(
                "linux BLE peripheral_id is invalid (all zeros)",
            ));
        }
        Ok(())
    }

    fn subscribe(&mut self, _settings: &BleRuntimeSettings) -> Result<(), BleBackendError> {
        Ok(())
    }

    fn write_probe(
        &mut self,
        payload: &[u8],
        _settings: &BleRuntimeSettings,
    ) -> Result<(), BleBackendError> {
        if !synthetic_probe_enabled() {
            return Err(BleBackendError::terminal(
                "linux BLE probe requires platform GATT I/O; synthetic loopback disabled (set LXMF_BLE_SYNTHETIC_PROBE=1 to bypass)",
            ));
        }
        self.probe_payload = payload.to_vec();
        Ok(())
    }

    fn read_probe_notification(
        &mut self,
        _settings: &BleRuntimeSettings,
    ) -> Result<Vec<u8>, BleBackendError> {
        if !synthetic_probe_enabled() {
            return Err(BleBackendError::terminal(
                "linux BLE probe notification unavailable without platform GATT I/O",
            ));
        }
        Ok(self.probe_payload.clone())
    }
}

pub(super) fn startup(
    iface: &InterfaceConfig,
    settings: &BleRuntimeSettings,
) -> Result<(), String> {
    let mut backend = LinuxBleBackend::default();
    let report = run_startup_lifecycle(&mut backend, settings)?;
    log_report(iface, settings, &report);
    Ok(())
}

fn log_report(iface: &InterfaceConfig, settings: &BleRuntimeSettings, report: &BleLifecycleReport) {
    eprintln!(
        "[daemon] ble_gatt configured (linux backend) name={} adapter={} peripheral_id={} service_uuid={} write_char_uuid={} notify_char_uuid={} mtu={} scan_timeout_ms={} connect_timeout_ms={} reconnect_backoff_ms={} max_reconnect_backoff_ms={} attempts={} transitions={}",
        iface.name.as_deref().unwrap_or("<unnamed>"),
        settings.adapter.as_deref().unwrap_or("<default>"),
        settings.peripheral_id,
        settings.service_uuid,
        settings.write_char_uuid,
        settings.notify_char_uuid,
        settings.mtu,
        settings.scan_timeout.as_millis(),
        settings.connect_timeout.as_millis(),
        settings.reconnect_backoff.as_millis(),
        settings.max_reconnect_backoff.as_millis(),
        report.attempts,
        report.transitions.len(),
    );
}
