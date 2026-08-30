use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use styrene_rbac::RbacPolicy;

static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
}

/// Config directory: $STYRENE_CONFIG_DIR or platform-appropriate default.
///
/// - Linux/macOS: `$XDG_CONFIG_HOME/styrene/` or `~/.config/styrene/`
/// - iOS/Android: Set `$STYRENE_CONFIG_DIR` to the app container path
///   before calling any config functions.
pub fn default_config_dir() -> PathBuf {
    if let Ok(d) = std::env::var("STYRENE_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("styrene");
    }
    home_dir().join(".config").join("styrene")
}

/// Data directory: $STYRENE_DATA_DIR or platform-appropriate default.
///
/// - Linux/macOS: `$XDG_DATA_HOME/styrene/` or `~/.local/share/styrene/`
/// - iOS/Android: Set `$STYRENE_DATA_DIR` to the app container path.
pub fn default_data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("STYRENE_DATA_DIR") {
        return PathBuf::from(d);
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("styrene");
    }
    home_dir().join(".local").join("share").join("styrene")
}

/// Platform paths for mobile embedding.
///
/// On iOS/Android, the host app sets these from the native container paths
/// before booting the daemon. On desktop, these are derived from XDG/HOME.
#[derive(Debug, Clone)]
pub struct PlatformPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl PlatformPaths {
    /// Create from explicit paths (mobile — host app provides container paths).
    pub fn new(config_dir: PathBuf, data_dir: PathBuf) -> Self {
        Self { config_dir, data_dir }
    }

    /// Create from platform defaults (desktop).
    pub fn from_defaults() -> Self {
        Self { config_dir: default_config_dir(), data_dir: default_data_dir() }
    }

    pub fn config_path(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn mobile_config_path(&self) -> PathBuf {
        self.config_dir.join("mobile.toml")
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("messages.db")
    }

    pub fn identity_path(&self) -> PathBuf {
        self.config_dir.join("identity")
    }

    pub fn pages_dir(&self) -> PathBuf {
        self.config_dir.join("pages")
    }

    /// Ensure directories exist.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.config_dir)?;
        fs::create_dir_all(&self.data_dir)?;
        Ok(())
    }
}

pub(crate) fn atomic_write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::other("path has no parent"))?;
    fs::create_dir_all(parent)?;
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("state");
    let temporary = loop {
        let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".{name}.tmp-{}-{sequence}", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(file) => break (candidate, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let (temporary_path, mut file) = temporary;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary_path, path)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Default config file path: ~/.config/styrene/config.toml
///
pub fn default_config_path() -> PathBuf {
    default_config_dir().join("config.toml")
}

/// Default database path: ~/.local/share/styrene/messages.db
pub fn default_db_path() -> PathBuf {
    default_data_dir().join("messages.db")
}

/// Default identity path: ~/.config/styrene/identity
pub fn default_identity_path() -> PathBuf {
    default_config_dir().join("identity")
}

/// Node role — determines what transport and protocol features are active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    /// Full node — runs transport, routes packets, maintains announce tables.
    #[default]
    FullNode,
    /// Propagation client — connects to a hub, fetches messages, no routing.
    /// Suitable for mobile/thin clients that wake periodically.
    PropagationClient,
    /// Hub — propagation store operator, routes and stores messages for clients.
    Hub,
}

impl NodeRole {
    /// Whether this role runs the full transport layer.
    pub fn runs_transport(&self) -> bool {
        matches!(self, Self::FullNode | Self::Hub)
    }

    /// Whether this role accepts inbound connections.
    pub fn accepts_inbound(&self) -> bool {
        matches!(self, Self::FullNode | Self::Hub)
    }

    /// Whether this role operates a propagation store.
    pub fn is_propagation_store(&self) -> bool {
        matches!(self, Self::Hub)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default)]
    pub interfaces: Vec<InterfaceConfig>,
    #[serde(default)]
    pub role: NodeRole,
    /// Whether this node forwards and retransmits transit RNS traffic.
    /// Defaults to true for existing full-node and hub configurations.
    #[serde(default)]
    pub transport_retransmit: Option<bool>,
    /// RBAC policy — role roster, blocked prefixes, default role.
    #[serde(default)]
    pub rbac: Option<RbacPolicy>,
    #[serde(default)]
    pub auto_reply: AutoReplySettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoReplySettingMode {
    #[default]
    Disabled,
    All,
    FirstOnly,
    Echo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoReplySettings {
    #[serde(default)]
    pub mode: AutoReplySettingMode,
    #[serde(default)]
    pub message: String,
    #[serde(default = "default_auto_reply_cooldown_secs")]
    pub cooldown_secs: u64,
}

impl Default for AutoReplySettings {
    fn default() -> Self {
        Self {
            mode: AutoReplySettingMode::Disabled,
            message: String::new(),
            cooldown_secs: default_auto_reply_cooldown_secs(),
        }
    }
}

const fn default_auto_reply_cooldown_secs() -> u64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceConfig {
    #[serde(rename = "type")]
    pub kind: String,
    pub enabled: Option<bool>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub name: Option<String>,
    #[serde(flatten)]
    pub rnode: RNodeInterfaceSettings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RNodeInterfaceSettings {
    pub device: Option<String>,
    pub baud_rate: Option<u32>,
    pub frequency_hz: Option<u32>,
    pub bandwidth_hz: Option<u32>,
    pub tx_power_dbm: Option<u8>,
    pub spreading_factor: Option<u8>,
    pub coding_rate: Option<u8>,
    pub reconnect_delay_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedRNodeInterface {
    pub device: String,
    pub baud_rate: u32,
    pub profile: rns_core::transport::iface::rnode::RNodeRadioProfile,
    pub reconnect_delay_ms: u64,
}

impl std::fmt::Display for NodeRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FullNode => write!(f, "full_node"),
            Self::PropagationClient => write!(f, "propagation_client"),
            Self::Hub => write!(f, "hub"),
        }
    }
}

impl DaemonConfig {
    pub fn transport_retransmit(&self) -> bool {
        self.transport_retransmit.unwrap_or(true)
    }

    pub fn from_toml(input: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(input)
    }

    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, std::io::Error> {
        let contents = fs::read_to_string(path)?;
        Self::from_toml(&contents)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
    }

    pub fn enabled_tcp_clients(&self) -> Vec<&InterfaceConfig> {
        self.interfaces
            .iter()
            .filter(|iface| iface.enabled.unwrap_or(false) && iface.kind == "tcp_client")
            .collect()
    }

    /// Return the configured TCP server bind address, if any.
    pub fn tcp_server_endpoint(&self) -> Option<String> {
        self.interfaces
            .iter()
            .find(|iface| iface.enabled.unwrap_or(false) && iface.kind == "tcp_server")
            .and_then(|iface| {
                let host = iface.host.as_deref().unwrap_or("0.0.0.0");
                let port = iface.port?;
                Some(format!("{}:{}", host, port))
            })
    }

    pub fn tcp_client_endpoints(&self) -> Vec<(String, u16)> {
        self.enabled_tcp_clients()
            .iter()
            .filter_map(|iface| {
                let host = iface.host.as_ref()?;
                let port = iface.port?;
                Some((host.clone(), port))
            })
            .collect()
    }

    pub fn rnode_interfaces(&self) -> Result<Vec<ValidatedRNodeInterface>, String> {
        let mut devices = std::collections::HashSet::new();
        self.interfaces
            .iter()
            .filter(|interface| interface.enabled.unwrap_or(false) && interface.kind == "rnode")
            .map(|interface| {
                let device = interface
                    .rnode
                    .device
                    .as_deref()
                    .map(str::trim)
                    .filter(|device| !device.is_empty())
                    .ok_or_else(|| "rnode interface requires device".to_string())?
                    .to_string();
                if !devices.insert(device.clone()) {
                    return Err("duplicate rnode device".to_string());
                }
                let profile = rns_core::transport::iface::rnode::RNodeRadioProfile::new(
                    interface
                        .rnode
                        .frequency_hz
                        .ok_or_else(|| "rnode interface requires frequency_hz".to_string())?,
                    interface
                        .rnode
                        .bandwidth_hz
                        .ok_or_else(|| "rnode interface requires bandwidth_hz".to_string())?,
                    interface
                        .rnode
                        .tx_power_dbm
                        .ok_or_else(|| "rnode interface requires tx_power_dbm".to_string())?,
                    interface
                        .rnode
                        .spreading_factor
                        .ok_or_else(|| "rnode interface requires spreading_factor".to_string())?,
                    interface
                        .rnode
                        .coding_rate
                        .ok_or_else(|| "rnode interface requires coding_rate".to_string())?,
                )
                .map_err(|error| error.to_string())?;
                Ok(ValidatedRNodeInterface {
                    device,
                    baud_rate: interface.rnode.baud_rate.unwrap_or(115_200),
                    profile,
                    reconnect_delay_ms: interface.rnode.reconnect_delay_ms.unwrap_or(3_000),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_role_defaults_to_full_node() {
        let config: DaemonConfig = toml::from_str("").unwrap();
        assert_eq!(config.role, NodeRole::FullNode);
        assert_eq!(config.auto_reply, AutoReplySettings::default());
    }

    #[test]
    fn parses_all_persistent_auto_reply_modes() {
        for (value, expected) in [
            ("disabled", AutoReplySettingMode::Disabled),
            ("all", AutoReplySettingMode::All),
            ("first_only", AutoReplySettingMode::FirstOnly),
            ("echo", AutoReplySettingMode::Echo),
        ] {
            let config =
                DaemonConfig::from_toml(&format!("[auto_reply]\nmode = \"{value}\"\n")).unwrap();
            assert_eq!(config.auto_reply.mode, expected);
        }
    }

    #[test]
    fn node_role_propagation_client() {
        let config: DaemonConfig = toml::from_str(r#"role = "propagation_client""#).unwrap();
        assert_eq!(config.role, NodeRole::PropagationClient);
        assert!(!config.role.runs_transport());
        assert!(!config.role.accepts_inbound());
        assert!(!config.role.is_propagation_store());
    }

    #[test]
    fn node_role_hub() {
        let config: DaemonConfig = toml::from_str(r#"role = "hub""#).unwrap();
        assert_eq!(config.role, NodeRole::Hub);
        assert!(config.role.runs_transport());
        assert!(config.role.accepts_inbound());
        assert!(config.role.is_propagation_store());
    }

    #[test]
    fn full_node_runs_transport() {
        assert!(NodeRole::FullNode.runs_transport());
        assert!(NodeRole::FullNode.accepts_inbound());
        assert!(!NodeRole::FullNode.is_propagation_store());
    }

    #[test]
    fn default_paths_use_styrene_layout() {
        // config dir: ~/.config/styrene/
        let config_dir = super::default_config_dir();
        assert!(config_dir.ends_with("styrene"), "config_dir={config_dir:?}");
        // data dir: ~/.local/share/styrene/
        let data_dir = super::default_data_dir();
        assert!(data_dir.ends_with("styrene"), "data_dir={data_dir:?}");
        // config file in config dir
        let config_path = super::default_config_path();
        assert!(config_path.ends_with("styrene/config.toml"), "config_path={config_path:?}");
        // db in data dir
        assert!(super::default_db_path().ends_with("styrene/messages.db"));
        // identity in config dir
        assert!(super::default_identity_path().ends_with("styrene/identity"));
    }

    #[test]
    fn node_role_display() {
        assert_eq!(NodeRole::FullNode.to_string(), "full_node");
        assert_eq!(NodeRole::PropagationClient.to_string(), "propagation_client");
        assert_eq!(NodeRole::Hub.to_string(), "hub");
    }
}
