//! Persistent interface configuration and serialized runtime application.
use super::config::ConfigService;
use crate::config::InterfaceConfig;
use crate::transport::mesh_transport::MeshTransport;
use rand_core::{OsRng, RngCore};
use rns_core::hash::AddressHash;
use rns_core::transport::iface::{InterfaceEndpoint, InterfaceSnapshot};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use styrene_ipc::{IpcError, types::*};

#[derive(Default)]
struct State {
    bindings: HashMap<String, AddressHash>,
    errors: HashMap<String, String>,
}
#[derive(Default)]
pub struct InterfaceService {
    state: tokio::sync::Mutex<State>,
}

fn endpoint(value: &InterfaceEndpoint) -> String {
    match value {
        InterfaceEndpoint::Socket(a) => a.to_string(),
        InterfaceEndpoint::Device { path, .. } => path.clone(),
    }
}
fn address(s: &InterfaceSettings) -> String {
    if s.host.contains(':') {
        format!("[{}]:{}", s.host, s.port)
    } else {
        format!("{}:{}", s.host, s.port)
    }
}
fn settings(c: &InterfaceConfig, index: usize) -> InterfaceSettings {
    let mut s = InterfaceSettings::default();
    s.id = c.id.clone().unwrap_or_else(|| format!("configured-{index}"));
    s.name = c.name.clone().unwrap_or_else(|| c.kind.clone());
    s.kind = c.kind.clone();
    s.host = c.host.clone().unwrap_or_default();
    s.port = c.port.unwrap_or_default();
    s.enabled = c.enabled.unwrap_or(false);
    s
}
fn editable(s: &InterfaceSettings) -> bool {
    matches!(s.kind.as_str(), "tcp_client" | "tcp_server")
}
fn validate(s: &InterfaceSettings) -> Result<(), IpcError> {
    if !editable(s) {
        return Err(IpcError::invalid_request(
            "This interface type cannot be edited by this runtime",
        ));
    }
    if s.name.trim().is_empty() || s.name.len() > 80 || s.name.chars().any(char::is_control) {
        return Err(IpcError::invalid_request(
            "Name must contain 1–80 characters without control characters",
        ));
    }
    if s.host.is_empty()
        || s.host.len() > 253
        || s.host.chars().any(|c| c.is_whitespace() || matches!(c, '/' | '@' | '[' | ']'))
    {
        return Err(IpcError::invalid_request(
            "Enter a hostname or IP address without a scheme, brackets, or port",
        ));
    }
    if s.host.contains(':') && s.host.parse::<std::net::Ipv6Addr>().is_err() {
        return Err(IpcError::invalid_request("Enter the port in its separate field"));
    }
    if s.kind == "tcp_server" && s.host.parse::<std::net::IpAddr>().is_err() {
        return Err(IpcError::invalid_request("A listener requires a local IP address"));
    }
    if s.kind == "tcp_client" && s.port == 0 {
        return Err(IpcError::invalid_request("Remote port must be 1–65535"));
    }
    Ok(())
}
fn revision(configs: &[InterfaceConfig]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // InterfaceConfig has no serialization-failing values.
    format!("{configs:?}").hash(&mut h);
    format!("{:016x}", h.finish())
}
impl InterfaceService {
    fn inventory(
        state: &mut State,
        config: &ConfigService,
        snapshots: &[InterfaceSnapshot],
    ) -> (InterfaceInventory, Vec<InterfaceConfig>) {
        let mut configs = config.interfaces();
        let mut used = HashSet::new();
        for (index, c) in configs.iter().enumerate() {
            let s = settings(c, index);
            if let Some(hash) = state.bindings.get(&s.id) {
                used.insert(*hash);
                continue;
            }
            if !s.enabled {
                continue;
            }
            if let Some(runtime) = snapshots.iter().find(|r| {
                r.parent.is_none()
                    && r.kind.as_str() == s.kind
                    && !used.contains(&r.hash)
                    && (s.kind != "tcp_server"
                        || r.local_endpoint.as_ref().is_some_and(|e| match e {
                            InterfaceEndpoint::Socket(a) => {
                                a.ip().to_string() == s.host && (s.port == 0 || a.port() == s.port)
                            }
                            _ => false,
                        }))
            }) {
                state.bindings.insert(s.id, runtime.hash);
                used.insert(runtime.hash);
            }
        }
        // Adopt pre-existing command-line listeners into the desired inventory on first edit.
        for runtime in snapshots.iter().filter(|r| r.parent.is_none() && !used.contains(&r.hash)) {
            let socket = if runtime.kind.as_str() == "tcp_client" {
                runtime.remote_endpoint.as_ref()
            } else {
                runtime.local_endpoint.as_ref()
            };
            if let Some(InterfaceEndpoint::Socket(socket)) = socket {
                let id = format!("runtime-{}", hex::encode(runtime.hash.as_slice()));
                let kind = runtime.kind.as_str().to_string();
                configs.push(InterfaceConfig {
                    id: Some(id.clone()),
                    kind: kind.clone(),
                    name: Some(if kind == "tcp_server" {
                        "Local TCP listener".into()
                    } else {
                        "TCP connection".into()
                    }),
                    host: Some(socket.ip().to_string()),
                    port: Some(socket.port()),
                    enabled: Some(true),
                    rnode: Default::default(),
                });
                state.bindings.insert(id, runtime.hash);
            }
        }
        let mut inventory = InterfaceInventory::default();
        inventory.revision = revision(&configs);
        for (index, c) in configs.iter().enumerate() {
            let s = settings(c, index);
            let runtime =
                state.bindings.get(&s.id).and_then(|h| snapshots.iter().find(|r| r.hash == *h));
            let mut entry = ManagedInterface::default();
            entry.editable = editable(&s) && config.config_path().is_some();
            entry.error = state.errors.get(&s.id).cloned();
            entry.state = if !s.enabled {
                "disabled".into()
            } else {
                runtime.map_or("not running", |r| r.state.as_str()).into()
            };
            if let Some(r) = runtime {
                entry.runtime_hash = Some(hex::encode(r.hash.as_slice()));
                entry.local_endpoint = r.local_endpoint.as_ref().map(endpoint);
                entry.remote_endpoint = r.remote_endpoint.as_ref().map(endpoint);
                entry.tx_bytes = r.tx_bytes;
                entry.rx_bytes = r.rx_bytes;
                entry.connected_peers = r.connected_peers;
            }
            if entry.error.is_none() && entry.state == "retrying" {
                entry.error = Some(if s.kind == "tcp_server" { "Listener could not bind; check the address and whether the port is already in use." } else { "Connection failed; automatic retry is active." }.into());
            }
            entry.settings = s;
            inventory.entries.push(entry);
        }
        (inventory, configs)
    }
    pub async fn list(
        &self,
        config: &ConfigService,
        transport: &dyn MeshTransport,
    ) -> InterfaceInventory {
        let mut state = self.state.lock().await;
        Self::inventory(&mut state, config, &transport.interface_snapshots().await).0
    }
    pub async fn mutate(
        &self,
        config: &ConfigService,
        transport: &dyn MeshTransport,
        request: InterfaceMutation,
    ) -> Result<InterfaceInventory, IpcError> {
        let mut state = self.state.lock().await;
        let (before, mut configs) =
            Self::inventory(&mut state, config, &transport.interface_snapshots().await);
        if request.expected_revision != before.revision {
            return Err(IpcError::Conflict {
                message: "Interfaces changed; refresh and retry".into(),
            });
        }
        let mut s = request.settings;
        let index = before.entries.iter().position(|e| e.settings.id == s.id);
        if request.action != "create" && index.is_none() {
            return Err(IpcError::invalid_request("Interface no longer exists"));
        }
        if let Some(index) = index
            && !before.entries[index].editable
        {
            return Err(IpcError::invalid_request("Interface is not editable"));
        }
        if request.action == "create" {
            if !s.id.is_empty() {
                return Err(IpcError::invalid_request("New interfaces must not supply an ID"));
            }
            if configs.len() >= 64 {
                return Err(IpcError::invalid_request(
                    "At most 64 configured interfaces are supported",
                ));
            }
            let mut bytes = [0u8; 16];
            OsRng
                .try_fill_bytes(&mut bytes)
                .map_err(|e| IpcError::Internal { message: e.to_string() })?;
            s.id = hex::encode(bytes);
        }
        if matches!(request.action.as_str(), "create" | "update") {
            validate(&s)?;
            if request.action == "update"
                && index.is_some_and(|i| before.entries[i].settings.kind != s.kind)
            {
                return Err(IpcError::invalid_request("Create a new interface to change its type"));
            }
            if before.entries.iter().any(|e| {
                e.settings.id != s.id
                    && e.settings.kind == s.kind
                    && e.settings.host == s.host
                    && e.settings.port == s.port
            }) {
                return Err(IpcError::invalid_request(
                    "An interface with this endpoint already exists",
                ));
            }
        } else if !matches!(request.action.as_str(), "delete" | "reconnect") {
            return Err(IpcError::invalid_request("Unknown interface action"));
        }
        // Persist IDs for imported configuration before any runtime changes.
        for (i, c) in configs.iter_mut().enumerate() {
            c.id = Some(before.entries[i].settings.id.clone());
        }
        let apply = match request.action.as_str() {
            "create" | "update" => {
                let c = InterfaceConfig {
                    id: Some(s.id.clone()),
                    kind: s.kind.clone(),
                    name: Some(s.name.trim().into()),
                    host: Some(s.host.clone()),
                    port: Some(s.port),
                    enabled: Some(s.enabled),
                    rnode: Default::default(),
                };
                if let Some(i) = index {
                    configs[i] = c;
                } else {
                    configs.push(c);
                }
                s.clone()
            }
            "delete" => {
                configs
                    .remove(index.ok_or_else(|| IpcError::invalid_request("Missing interface"))?);
                s.clone()
            }
            _ => before.entries
                [index.ok_or_else(|| IpcError::invalid_request("Missing interface"))?]
            .settings
            .clone(),
        };
        if request.action == "reconnect" && !apply.enabled {
            return Err(IpcError::invalid_request("Enable the interface before reconnecting"));
        }
        config.replace_interfaces(configs).map_err(|e| IpcError::Internal {
            message: format!("Configuration was not saved: {e}"),
        })?;
        state.errors.remove(&s.id);
        if let Some(hash) = state.bindings.remove(&s.id)
            && let Err(e) = transport.stop_managed_interface(&hash).await
        {
            state.errors.insert(s.id.clone(), format!("Saved, but could not stop runtime: {e}"));
        }
        if request.action != "delete" && apply.enabled && !state.errors.contains_key(&s.id) {
            match transport.start_tcp_interface(&apply.kind, &address(&apply)).await {
                Ok(hash) => {
                    state.bindings.insert(s.id.clone(), hash);
                }
                Err(e) => {
                    state
                        .errors
                        .insert(s.id.clone(), format!("Saved, but could not start runtime: {e}"));
                }
            }
        }
        Ok(Self::inventory(&mut state, config, &transport.interface_snapshots().await).0)
    }
}
