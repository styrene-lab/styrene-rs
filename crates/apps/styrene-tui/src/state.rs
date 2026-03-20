//! Application state — shared types that will move to styrene-app-core.

use rns_core::identity::PrivateIdentity;

/// UI-friendly identity summary.
#[derive(Clone, Debug)]
pub struct IdentityInfo {
    pub hash_hex: String,
    pub public_key_hex: String,
    pub signing_key_hex: String,
}

/// Placeholder mesh status.
#[derive(Clone, Debug, Default)]
pub struct MeshStatus {
    pub interfaces: usize,
    pub known_paths: usize,
    pub transport_active: bool,
    pub announces_seen: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Identity,
    Mesh,
    Micron,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Identity, Tab::Mesh, Tab::Micron];

    pub fn title(&self) -> &'static str {
        match self {
            Tab::Identity => "Identity",
            Tab::Mesh => "Mesh Status",
            Tab::Micron => "Micron",
        }
    }
}

pub struct AppState {
    pub identity: IdentityInfo,
    pub mesh: MeshStatus,
    pub active_tab: Tab,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            identity: generate_identity_info(),
            mesh: MeshStatus::default(),
            active_tab: Tab::Identity,
        }
    }

    pub fn regenerate_identity(&mut self) {
        self.identity = generate_identity_info();
    }

    pub fn next_tab(&mut self) {
        let idx = Tab::ALL.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        self.active_tab = Tab::ALL[(idx + 1) % Tab::ALL.len()];
    }

    pub fn prev_tab(&mut self) {
        let idx = Tab::ALL.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        self.active_tab = Tab::ALL[(idx + Tab::ALL.len() - 1) % Tab::ALL.len()];
    }
}

fn generate_identity_info() -> IdentityInfo {
    let mut rng = rand_core::OsRng;
    let identity = PrivateIdentity::new_from_rand(&mut rng);
    let public = identity.as_identity();

    IdentityInfo {
        hash_hex: hex::encode(public.address_hash.as_slice()),
        public_key_hex: hex::encode(public.public_key_bytes()),
        signing_key_hex: hex::encode(public.verifying_key_bytes()),
    }
}
