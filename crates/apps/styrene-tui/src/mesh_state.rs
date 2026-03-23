#![allow(dead_code)]
//! Mesh data model — peer, link, and protocol activity records.
//!
//! This is the shared data substrate that all Phase 4+ panels read from:
//!   - topology.rs (peer tree sidebar)
//!   - signal.rs   (RTT wave strings + activity feed)
//!   - footer.rs   (derived counts)
//!   - conversation.rs (source/dest display names)
//!
//! Phase 5: this will be populated live from styrened-rs RPC events.
//! Right now it is populated by the demo_announce / demo_link methods.

use std::collections::VecDeque;
use std::time::Instant;

/// Maximum protocol activity entries to retain.
pub const ACTIVITY_RING_LEN: usize = 64;

// ─── Peer ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerStatus {
    /// Announced recently (within 5 minutes).
    Online,
    /// Last announce was >5 minutes ago.
    Stale,
    /// Explicitly unpeered or announce timed out completely.
    Offline,
}

impl PeerStatus {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Online => "◉",
            Self::Stale => "◎",
            Self::Offline => "○",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PeerRecord {
    /// Full 32-hex-char destination hash.
    pub hash: String,
    /// Display name from announce app_data, if decoded.
    pub name: Option<String>,
    /// Epoch seconds of first observe.
    pub first_seen: u64,
    /// Epoch seconds of most recent announce.
    pub last_seen: u64,
    /// Hop count from last announce packet.
    pub hop_count: u8,
    /// Current online/stale/offline status.
    pub status: PeerStatus,
    /// IDs of active links to this peer.
    pub link_ids: Vec<String>,
}

impl PeerRecord {
    pub fn new(hash: String, name: Option<String>, now: u64) -> Self {
        Self {
            hash,
            name,
            first_seen: now,
            last_seen: now,
            hop_count: 1,
            status: PeerStatus::Online,
            link_ids: Vec::new(),
        }
    }

    /// Short display hash (first 8 chars).
    pub fn short_hash(&self) -> &str {
        &self.hash[..self.hash.len().min(8)]
    }

    /// Display label: name if available, else short hash.
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or_else(|| self.short_hash())
    }

    /// Update last_seen and recompute status.
    pub fn touch(&mut self, now: u64, hop_count: u8) {
        self.last_seen = now;
        self.hop_count = hop_count;
        self.status = PeerStatus::Online;
    }

    /// Age in seconds since last announce.
    pub fn age_secs(&self, now: u64) -> u64 {
        now.saturating_sub(self.last_seen)
    }
}

// ─── Link ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkStatus {
    Pending,
    Active,
    Stale,
    Closed,
}

impl LinkStatus {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Pending => "◌",
            Self::Active => "⟺",
            Self::Stale => "⟳",
            Self::Closed => "✕",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active | Self::Stale)
    }
}

#[derive(Debug, Clone)]
pub struct LinkRecord {
    /// Short hex ID derived from the link request packet hash.
    pub id: String,
    /// Destination peer hash this link reaches.
    pub peer_hash: String,
    /// Cached peer name at link establishment.
    pub peer_name: Option<String>,
    /// Current link lifecycle state.
    pub status: LinkStatus,
    /// Round-trip time in milliseconds (updated from RTT packets).
    pub rtt_ms: f64,
    /// Epoch seconds when link was established.
    pub established_at: u64,
    /// Epoch seconds of last inbound packet.
    pub last_packet_at: u64,
    /// Keepalive period in seconds (RTT-derived).
    pub keepalive_secs: f64,
    /// Phase for RTT wave animation (advances each frame).
    pub wave_phase: f64,
    /// Wave amplitude — plucked on packet receipt, decays to resting.
    pub wave_amplitude: f64,
}

impl LinkRecord {
    pub fn new(id: String, peer_hash: String, peer_name: Option<String>, now: u64) -> Self {
        Self {
            id,
            peer_hash,
            peer_name,
            status: LinkStatus::Active,
            rtt_ms: 0.0,
            established_at: now,
            last_packet_at: now,
            keepalive_secs: 360.0,
            wave_phase: 0.0,
            wave_amplitude: 1.0, // Start plucked
        }
    }

    /// Short ID for display (first 8 chars).
    pub fn short_id(&self) -> &str {
        &self.id[..self.id.len().min(8)]
    }

    /// Display label for this link.
    pub fn label(&self) -> String {
        match &self.peer_name {
            Some(n) => n.clone(),
            None => self.short_id().to_string(),
        }
    }

    /// Pluck the wave — called when a packet arrives on this link.
    pub fn pluck(&mut self) {
        self.wave_amplitude = 1.0;
        self.last_packet_at = epoch_secs();
    }

    /// Advance wave animation by dt seconds.
    pub fn tick_wave(&mut self, dt: f64) {
        let speed = if self.rtt_ms > 0.0 {
            std::f64::consts::PI * 2.0 / (self.rtt_ms / 1000.0).max(0.1)
        } else {
            4.0
        };
        self.wave_phase = (self.wave_phase + speed * dt) % (std::f64::consts::PI * 2.0);
        self.wave_amplitude = (self.wave_amplitude - dt * 0.8).max(0.05);
    }

    /// Current wave displacement at a given column position (0.0–1.0).
    pub fn wave_at(&self, t: f64) -> f64 {
        (self.wave_phase + t * std::f64::consts::PI * 2.0).sin() * self.wave_amplitude
    }
}

// ─── Protocol Activity (ring buffer) ─────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ActivityKind {
    Announce,
    LinkUp,
    LinkDown,
    Receipt,
    ResourceStart,
    ResourceDone,
    PropagationSync,
    InboundMessage,
    OutboundMessage,
}

impl ActivityKind {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Announce => "⬡",
            Self::LinkUp => "⟺",
            Self::LinkDown => "✕",
            Self::Receipt => "✓",
            Self::ResourceStart => "⬇",
            Self::ResourceDone => "●",
            Self::PropagationSync => "⟳",
            Self::InboundMessage => "←",
            Self::OutboundMessage => "→",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActivityEntry {
    pub kind: ActivityKind,
    pub peer_label: String,
    pub detail: String,
    pub when: Instant,
}

impl ActivityEntry {
    pub fn new(kind: ActivityKind, peer_label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            kind,
            peer_label: peer_label.into(),
            detail: detail.into(),
            when: Instant::now(),
        }
    }

    pub fn age_secs(&self) -> f64 {
        self.when.elapsed().as_secs_f64()
    }
}

/// Ring buffer of recent protocol events.
pub struct ActivityLog {
    entries: VecDeque<ActivityEntry>,
    capacity: usize,
}

impl ActivityLog {
    pub fn new() -> Self {
        Self { entries: VecDeque::new(), capacity: ACTIVITY_RING_LEN }
    }

    pub fn push(&mut self, entry: ActivityEntry) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_back();
        }
        self.entries.push_front(entry);
    }

    pub fn entries(&self) -> impl Iterator<Item = &ActivityEntry> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

pub fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
