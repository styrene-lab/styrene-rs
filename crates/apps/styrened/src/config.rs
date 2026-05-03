use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use styrene_rbac::RbacPolicy;

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
        let toml = self.config_dir.join("config.toml");
        if toml.exists() {
            return toml;
        }
        let yaml = self.config_dir.join("config.yaml");
        if yaml.exists() {
            return yaml;
        }
        toml
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

/// Default config file path: ~/.config/styrene/config.toml
///
/// Falls back to `config.yaml` if the TOML file does not exist (migration
/// path from Python's styrened which used YAML). The Rust daemon parses
/// TOML regardless of the file extension.
pub fn default_config_path() -> PathBuf {
    let toml_path = default_config_dir().join("config.toml");
    if toml_path.exists() {
        return toml_path;
    }
    // Legacy fallback — Python daemon used config.yaml
    let yaml_path = default_config_dir().join("config.yaml");
    if yaml_path.exists() {
        return yaml_path;
    }
    // Default to .toml for new installs
    toml_path
}

/// Default database path: ~/.local/share/styrene/messages.db
pub fn default_db_path() -> PathBuf {
    default_data_dir().join("messages.db")
}

/// Default identity path: ~/.config/styrene/identity
/// Matches Python's styrened.paths.identity_file().
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
    /// RBAC policy — role roster, blocked prefixes, default role.
    #[serde(default)]
    pub rbac: Option<RbacPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceConfig {
    #[serde(rename = "type")]
    pub kind: String,
    pub enabled: Option<bool>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub name: Option<String>,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_role_defaults_to_full_node() {
        let config: DaemonConfig = toml::from_str("").unwrap();
        assert_eq!(config.role, NodeRole::FullNode);
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
    fn default_paths_match_python_layout() {
        // config dir: ~/.config/styrene/
        let config_dir = super::default_config_dir();
        assert!(config_dir.ends_with("styrene"), "config_dir={config_dir:?}");
        // data dir: ~/.local/share/styrene/
        let data_dir = super::default_data_dir();
        assert!(data_dir.ends_with("styrene"), "data_dir={data_dir:?}");
        // config file in config dir (defaults to .toml for new installs)
        let config_path = super::default_config_path();
        assert!(
            config_path.ends_with("styrene/config.toml")
                || config_path.ends_with("styrene/config.yaml"),
            "config_path={config_path:?}"
        );
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
