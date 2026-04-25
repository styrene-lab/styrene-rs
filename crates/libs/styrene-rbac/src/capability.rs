//! Capability strings and per-tier capability sets.
//!
//! Capabilities are dot-separated strings (`chat.send`, `rpc.exec`).
//! Each role tier has a cumulative set — higher tiers inherit all
//! capabilities from tiers below.

/// Capability string constants.
///
/// Organized by the minimum tier required to hold each capability.
/// Orthogonal capabilities (`vpn.handshake`, `relay.reject`) are not
/// included in any tier and require explicit grants.
pub struct Capability;

impl Capability {
    // ── Peer (10) ──────────────────────────────────────────────
    pub const CHAT_SEND: &str = "chat.send";
    pub const CHAT_RECEIVE: &str = "chat.receive";
    pub const PAGE_BROWSE: &str = "page.browse";
    pub const RPC_PING: &str = "rpc.ping";
    pub const RPC_STATUS: &str = "rpc.status";
    pub const DATALINK_PING: &str = "datalink.ping";
    pub const DATALINK_META: &str = "datalink.meta";
    pub const DATALINK_INFO: &str = "datalink.info";
    pub const DATALINK_STATUS: &str = "datalink.status";
    pub const RELAY_REQUEST: &str = "relay.request";
    pub const RELAY_LIST: &str = "relay.list";
    pub const RELAY_TEARDOWN: &str = "relay.teardown";
    pub const RELAY_ACCEPT: &str = "relay.accept";

    // ── Monitor (20) ──────────────────────────────────────────
    pub const RPC_INBOX_READ: &str = "rpc.inbox_read";
    pub const WEB_READ: &str = "web.read";
    pub const DATALINK_ESTABLISH: &str = "datalink.establish";
    pub const DATALINK_SPEEDTEST: &str = "datalink.speedtest";

    // ── Operator (30) ─────────────────────────────────────────
    pub const RPC_CONFIG_UPDATE: &str = "rpc.config_update";
    pub const TERMINAL_RESTRICTED: &str = "terminal.restricted";
    pub const WEB_WRITE: &str = "web.write";
    pub const RELAY_REQUEST_PERMANENT: &str = "relay.request_permanent";
    pub const RELAY_ACCEPT_PERMANENT: &str = "relay.accept_permanent";
    pub const RELAY_PRIORITIZE: &str = "relay.prioritize";
    pub const RELAY_BRIDGE: &str = "relay.bridge";

    // ── Admin (40) ────────────────────────────────────────────
    pub const RPC_EXEC: &str = "rpc.exec";
    pub const RPC_REBOOT: &str = "rpc.reboot";
    pub const RPC_SELF_UPDATE: &str = "rpc.self_update";
    pub const TERMINAL_FULL: &str = "terminal.full";
    pub const ADAPTER_PROVISION: &str = "adapter.provision";
    pub const RELAY_ADMIN: &str = "relay.admin";

    // ── Orthogonal (explicit grant only) ──────────────────────
    pub const VPN_HANDSHAKE: &str = "vpn.handshake";
    pub const RELAY_REJECT: &str = "relay.reject";

    // ── Agent-to-agent (aether) ───────────────────────────────
    pub const AETHER_DELEGATE: &str = "aether.delegate";
    pub const AETHER_QUERY: &str = "aether.query";
    pub const AETHER_REPORT: &str = "aether.report";
}

/// All known capability strings (for validation).
pub const ALL_CAPABILITIES: &[&str] = &[
    // Peer
    Capability::CHAT_SEND,
    Capability::CHAT_RECEIVE,
    Capability::PAGE_BROWSE,
    Capability::RPC_PING,
    Capability::RPC_STATUS,
    Capability::DATALINK_PING,
    Capability::DATALINK_META,
    Capability::DATALINK_INFO,
    Capability::DATALINK_STATUS,
    Capability::RELAY_REQUEST,
    Capability::RELAY_LIST,
    Capability::RELAY_TEARDOWN,
    Capability::RELAY_ACCEPT,
    // Monitor
    Capability::RPC_INBOX_READ,
    Capability::WEB_READ,
    Capability::DATALINK_ESTABLISH,
    Capability::DATALINK_SPEEDTEST,
    // Operator
    Capability::RPC_CONFIG_UPDATE,
    Capability::TERMINAL_RESTRICTED,
    Capability::WEB_WRITE,
    Capability::RELAY_REQUEST_PERMANENT,
    Capability::RELAY_ACCEPT_PERMANENT,
    Capability::RELAY_PRIORITIZE,
    Capability::RELAY_BRIDGE,
    // Admin
    Capability::RPC_EXEC,
    Capability::RPC_REBOOT,
    Capability::RPC_SELF_UPDATE,
    Capability::TERMINAL_FULL,
    Capability::ADAPTER_PROVISION,
    Capability::RELAY_ADMIN,
    // Orthogonal
    Capability::VPN_HANDSHAKE,
    Capability::RELAY_REJECT,
    // Aether
    Capability::AETHER_DELEGATE,
    Capability::AETHER_QUERY,
    Capability::AETHER_REPORT,
];

/// Capabilities granted at the Peer tier (cumulative base).
pub const PEER_CAPS: &[&str] = &[
    Capability::CHAT_SEND,
    Capability::CHAT_RECEIVE,
    Capability::PAGE_BROWSE,
    Capability::RPC_PING,
    Capability::RPC_STATUS,
    Capability::DATALINK_PING,
    Capability::DATALINK_META,
    Capability::DATALINK_INFO,
    Capability::DATALINK_STATUS,
    Capability::RELAY_REQUEST,
    Capability::RELAY_LIST,
    Capability::RELAY_TEARDOWN,
    Capability::RELAY_ACCEPT,
    // Aether: all peers can query and report
    Capability::AETHER_QUERY,
    Capability::AETHER_REPORT,
];

/// Capabilities granted at the Monitor tier (includes Peer).
pub const MONITOR_CAPS: &[&str] = &[
    // Peer
    Capability::CHAT_SEND,
    Capability::CHAT_RECEIVE,
    Capability::PAGE_BROWSE,
    Capability::RPC_PING,
    Capability::RPC_STATUS,
    Capability::DATALINK_PING,
    Capability::DATALINK_META,
    Capability::DATALINK_INFO,
    Capability::DATALINK_STATUS,
    Capability::RELAY_REQUEST,
    Capability::RELAY_LIST,
    Capability::RELAY_TEARDOWN,
    Capability::RELAY_ACCEPT,
    Capability::AETHER_QUERY,
    Capability::AETHER_REPORT,
    // Monitor
    Capability::RPC_INBOX_READ,
    Capability::WEB_READ,
    Capability::DATALINK_ESTABLISH,
    Capability::DATALINK_SPEEDTEST,
];

/// Capabilities granted at the Operator tier (includes Monitor).
pub const OPERATOR_CAPS: &[&str] = &[
    // Peer
    Capability::CHAT_SEND,
    Capability::CHAT_RECEIVE,
    Capability::PAGE_BROWSE,
    Capability::RPC_PING,
    Capability::RPC_STATUS,
    Capability::DATALINK_PING,
    Capability::DATALINK_META,
    Capability::DATALINK_INFO,
    Capability::DATALINK_STATUS,
    Capability::RELAY_REQUEST,
    Capability::RELAY_LIST,
    Capability::RELAY_TEARDOWN,
    Capability::RELAY_ACCEPT,
    Capability::AETHER_QUERY,
    Capability::AETHER_REPORT,
    // Monitor
    Capability::RPC_INBOX_READ,
    Capability::WEB_READ,
    Capability::DATALINK_ESTABLISH,
    Capability::DATALINK_SPEEDTEST,
    // Operator
    Capability::RPC_CONFIG_UPDATE,
    Capability::TERMINAL_RESTRICTED,
    Capability::WEB_WRITE,
    Capability::RELAY_REQUEST_PERMANENT,
    Capability::RELAY_ACCEPT_PERMANENT,
    Capability::RELAY_PRIORITIZE,
    Capability::RELAY_BRIDGE,
    // Operator can delegate
    Capability::AETHER_DELEGATE,
];

/// Capabilities granted at the Admin tier (includes Operator).
/// Note: `vpn.handshake` and `relay.reject` are intentionally excluded —
/// they are orthogonal and require explicit grants.
pub const ADMIN_CAPS: &[&str] = &[
    // Peer
    Capability::CHAT_SEND,
    Capability::CHAT_RECEIVE,
    Capability::PAGE_BROWSE,
    Capability::RPC_PING,
    Capability::RPC_STATUS,
    Capability::DATALINK_PING,
    Capability::DATALINK_META,
    Capability::DATALINK_INFO,
    Capability::DATALINK_STATUS,
    Capability::RELAY_REQUEST,
    Capability::RELAY_LIST,
    Capability::RELAY_TEARDOWN,
    Capability::RELAY_ACCEPT,
    Capability::AETHER_QUERY,
    Capability::AETHER_REPORT,
    // Monitor
    Capability::RPC_INBOX_READ,
    Capability::WEB_READ,
    Capability::DATALINK_ESTABLISH,
    Capability::DATALINK_SPEEDTEST,
    // Operator
    Capability::RPC_CONFIG_UPDATE,
    Capability::TERMINAL_RESTRICTED,
    Capability::WEB_WRITE,
    Capability::RELAY_REQUEST_PERMANENT,
    Capability::RELAY_ACCEPT_PERMANENT,
    Capability::RELAY_PRIORITIZE,
    Capability::RELAY_BRIDGE,
    Capability::AETHER_DELEGATE,
    // Admin
    Capability::RPC_EXEC,
    Capability::RPC_REBOOT,
    Capability::RPC_SELF_UPDATE,
    Capability::TERMINAL_FULL,
    Capability::ADAPTER_PROVISION,
    Capability::RELAY_ADMIN,
];

use crate::Role;

/// Get the capability set for a given role.
pub fn capabilities_for_role(role: Role) -> &'static [&'static str] {
    match role {
        Role::Blocked | Role::None => &[],
        Role::Peer => PEER_CAPS,
        Role::Monitor => MONITOR_CAPS,
        Role::Operator => OPERATOR_CAPS,
        Role::Admin => ADMIN_CAPS,
    }
}

/// Check whether a capability string is a known capability.
pub fn is_valid_capability(cap: &str) -> bool {
    ALL_CAPABILITIES.contains(&cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cumulative_inclusion() {
        // Every peer cap must appear in monitor, operator, and admin sets.
        for cap in PEER_CAPS {
            assert!(MONITOR_CAPS.contains(cap), "monitor missing peer cap: {cap}");
            assert!(OPERATOR_CAPS.contains(cap), "operator missing peer cap: {cap}");
            assert!(ADMIN_CAPS.contains(cap), "admin missing peer cap: {cap}");
        }
        for cap in MONITOR_CAPS {
            assert!(OPERATOR_CAPS.contains(cap), "operator missing monitor cap: {cap}");
            assert!(ADMIN_CAPS.contains(cap), "admin missing monitor cap: {cap}");
        }
        for cap in OPERATOR_CAPS {
            assert!(ADMIN_CAPS.contains(cap), "admin missing operator cap: {cap}");
        }
    }

    #[test]
    fn orthogonal_not_in_admin() {
        assert!(!ADMIN_CAPS.contains(&Capability::VPN_HANDSHAKE));
        assert!(!ADMIN_CAPS.contains(&Capability::RELAY_REJECT));
    }

    #[test]
    fn aether_caps_in_hierarchy() {
        assert!(PEER_CAPS.contains(&Capability::AETHER_QUERY));
        assert!(PEER_CAPS.contains(&Capability::AETHER_REPORT));
        assert!(!PEER_CAPS.contains(&Capability::AETHER_DELEGATE));
        assert!(OPERATOR_CAPS.contains(&Capability::AETHER_DELEGATE));
    }

    #[test]
    fn all_capabilities_contains_everything() {
        for cap in ADMIN_CAPS {
            assert!(ALL_CAPABILITIES.contains(cap), "ALL missing admin cap: {cap}");
        }
        assert!(ALL_CAPABILITIES.contains(&Capability::VPN_HANDSHAKE));
        assert!(ALL_CAPABILITIES.contains(&Capability::RELAY_REJECT));
    }

    #[test]
    fn capabilities_for_blocked_is_empty() {
        assert!(capabilities_for_role(Role::Blocked).is_empty());
        assert!(capabilities_for_role(Role::None).is_empty());
    }
}
