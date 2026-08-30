use std::fs;
use std::path::{Path, PathBuf};
use styrened::config::{DaemonConfig, InterfaceConfig, NodeRole};
use tempfile::NamedTempFile;

#[test]
fn parses_tcp_client_interface() {
    let input = r#"
interfaces = [
  { type = "tcp_client", enabled = true, host = "rmap.world", port = 4242, name = "Public RMap" }
]
"#;
    let cfg = DaemonConfig::from_toml(input).expect("parse");
    assert_eq!(cfg.interfaces.len(), 1);
    let iface = &cfg.interfaces[0];
    assert_eq!(iface.name.as_deref(), Some("Public RMap"));
    assert_eq!(iface.host.as_deref(), Some("rmap.world"));
    assert_eq!(iface.port, Some(4242));
    assert!(iface.enabled.unwrap_or(false));
}

#[test]
fn filters_enabled_tcp_clients() {
    let cfg = DaemonConfig {
        role: Default::default(),
        transport_retransmit: None,
        rbac: None,
        interfaces: vec![
            InterfaceConfig {
                kind: "tcp_client".into(),
                enabled: Some(true),
                host: Some("rmap.world".into()),
                port: Some(4242),
                name: None,
                rnode: Default::default(),
            },
            InterfaceConfig {
                kind: "tcp_client".into(),
                enabled: Some(false),
                host: Some("example.com".into()),
                port: Some(1),
                name: None,
                rnode: Default::default(),
            },
        ],
    };
    let endpoints = cfg.tcp_client_endpoints();
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].0, "rmap.world");
    assert_eq!(endpoints[0].1, 4242);
}

#[test]
fn transit_retransmit_defaults_on_and_can_be_disabled() {
    let default = DaemonConfig::from_toml("").expect("parse default config");
    assert!(default.transport_retransmit());

    let endpoint =
        DaemonConfig::from_toml("transport_retransmit = false").expect("parse endpoint config");
    assert!(!endpoint.transport_retransmit());
}

#[test]
fn parses_and_validates_only_enabled_rnode_interfaces() {
    let cfg = DaemonConfig::from_toml(
        r#"
[[interfaces]]
type = "rnode"
enabled = true
device = "/dev/test-rnode"
frequency_hz = 915000000
bandwidth_hz = 125000
tx_power_dbm = 17
spreading_factor = 7
coding_rate = 5

[[interfaces]]
type = "rnode"
enabled = false
"#,
    )
    .expect("parse RNode interface");

    let interfaces = cfg.rnode_interfaces().expect("validate RNode interface");
    assert_eq!(interfaces.len(), 1);
    assert_eq!(interfaces[0].baud_rate, 115_200);
    assert_eq!(interfaces[0].profile.frequency_hz, 915_000_000);
}

#[test]
fn invalid_rnode_profile_fails_validation() {
    let cfg = DaemonConfig::from_toml(
        r#"
[[interfaces]]
type = "rnode"
enabled = true
device = "/dev/test-rnode"
frequency_hz = 915000000
bandwidth_hz = 125000
tx_power_dbm = 38
spreading_factor = 7
coding_rate = 5
"#,
    )
    .expect("parse RNode interface");

    assert_eq!(cfg.rnode_interfaces().unwrap_err(), "invalid RNode tx power: 38");
}

#[test]
fn loads_config_from_file() {
    let input = r#"
interfaces = [
  { type = "tcp_client", enabled = true, host = "rmap.world", port = 4242 }
]
"#;
    let file = NamedTempFile::new().expect("temp file");
    fs::write(file.path(), input).expect("write");

    let cfg = DaemonConfig::from_path(file.path()).expect("load");
    let endpoints = cfg.tcp_client_endpoints();
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].0, "rmap.world");
    assert_eq!(endpoints[0].1, 4242);
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

#[test]
fn checked_in_hub_deployments_use_hub_role_and_tcp_listener() {
    for relative_path in ["deploy/hub.toml", "tests/mesh/configs/hub.toml"] {
        let path = workspace_root().join(relative_path);
        let config = DaemonConfig::from_path(&path)
            .unwrap_or_else(|error| panic!("failed to load {}: {error}", path.display()));

        assert_eq!(config.role, NodeRole::Hub, "{relative_path} must run in hub mode");
        assert_eq!(
            config.tcp_server_endpoint().as_deref(),
            Some("0.0.0.0:4242"),
            "{relative_path} must expose the standard hub transport"
        );
    }
}
