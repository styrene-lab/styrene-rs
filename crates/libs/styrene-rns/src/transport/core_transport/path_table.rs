use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant, SystemTime},
};

use crate::transport::error::RnsError;
use crate::{
    hash::{AddressHash, Hash},
    packet::{DestinationType, Header, HeaderType, IfacFlag, Packet, PacketType, PropagationType},
};
use rmp::encode::write_array_len;

use crate::transport::iface::InterfaceMode;

pub const DEFAULT_PATH_LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub const ACCESS_POINT_PATH_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
pub const ROAMING_PATH_LIFETIME: Duration = Duration::from_secs(6 * 60 * 60);
const LOST_DESTINATION_CAPACITY: usize = 256;
const RANDOM_BLOB_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteEventKind {
    Discovered,
    Lost,
    Rediscovered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteLossReason {
    Expired,
    InterfaceUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RouteEvent {
    pub kind: RouteEventKind,
    pub route: PathSnapshot,
    pub loss_reason: Option<RouteLossReason>,
    pub occurred_at: SystemTime,
}

pub struct PathEntry {
    pub timestamp: Instant,
    pub observed_at: SystemTime,
    pub lifetime: Duration,
    pub replacement_expires_at: Instant,
    pub random_blobs: VecDeque<[u8; crate::destination::RAND_HASH_LENGTH]>,
    pub received_from: AddressHash,
    pub hops: u8,
    pub iface: AddressHash,
    pub packet_hash: Hash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathSnapshot {
    pub destination: AddressHash,
    pub hops: u8,
    pub received_from: AddressHash,
    pub iface: AddressHash,
    pub age: Duration,
    pub observed_at: SystemTime,
    pub lifetime: Duration,
    pub expires_at: SystemTime,
}

impl PathEntry {
    fn snapshot(&self, destination: AddressHash, now: Instant) -> PathSnapshot {
        PathSnapshot {
            destination,
            hops: self.hops,
            received_from: self.received_from,
            iface: self.iface,
            age: now.saturating_duration_since(self.timestamp),
            observed_at: self.observed_at,
            lifetime: self.lifetime,
            expires_at: self.observed_at + self.lifetime,
        }
    }

    fn is_expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.timestamp) > self.lifetime
    }
}

pub struct PathTable {
    map: HashMap<AddressHash, PathEntry>,
    lost_destinations: VecDeque<AddressHash>,
}

impl PathTable {
    pub fn new() -> Self {
        Self { map: HashMap::new(), lost_destinations: VecDeque::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn to_msgpack(&self) -> Result<Vec<u8>, RnsError> {
        if !self.map.is_empty() {
            return Err(RnsError::InvalidArgument);
        }

        let mut out = Vec::new();
        write_array_len(&mut out, 0).map_err(|_| RnsError::InvalidArgument)?;
        Ok(out)
    }

    pub fn get(&self, destination: &AddressHash) -> Option<&PathEntry> {
        self.map.get(destination)
    }

    /// Iterate over all path table entries.
    pub fn entries(&self) -> impl Iterator<Item = (&AddressHash, &PathEntry)> {
        self.map.iter()
    }

    pub fn snapshot(&self, destination: &AddressHash, now: Instant) -> Option<PathSnapshot> {
        self.map.get(destination).map(|entry| entry.snapshot(*destination, now))
    }

    pub fn snapshots(&self, now: Instant) -> Vec<PathSnapshot> {
        self.map.iter().map(|(destination, entry)| entry.snapshot(*destination, now)).collect()
    }

    pub fn next_hop_full(&self, destination: &AddressHash) -> Option<(AddressHash, AddressHash)> {
        self.map.get(destination).map(|entry| (entry.received_from, entry.iface))
    }

    pub fn next_hop_iface(&self, destination: &AddressHash) -> Option<AddressHash> {
        self.map.get(destination).map(|entry| entry.iface)
    }

    pub fn next_hop(&self, destination: &AddressHash) -> Option<AddressHash> {
        self.map.get(destination).map(|entry| entry.received_from)
    }

    pub fn handle_announce(
        &mut self,
        announce: &Packet,
        transport_id: Option<AddressHash>,
        iface: AddressHash,
        mode: InterfaceMode,
        random_blob: [u8; crate::destination::RAND_HASH_LENGTH],
    ) -> Option<RouteEvent> {
        self.handle_announce_at(
            announce,
            transport_id,
            iface,
            mode,
            random_blob,
            (Instant::now(), SystemTime::now()),
        )
    }

    fn handle_announce_at(
        &mut self,
        announce: &Packet,
        transport_id: Option<AddressHash>,
        iface: AddressHash,
        mode: InterfaceMode,
        random_blob: [u8; crate::destination::RAND_HASH_LENGTH],
        clock: (Instant, SystemTime),
    ) -> Option<RouteEvent> {
        let (now, observed_at) = clock;
        let hops = announce.header.hops;
        let received_from = transport_id.unwrap_or(announce.destination);

        let random_blobs = if let Some(existing_entry) = self.map.get(&announce.destination) {
            let seen = existing_entry.random_blobs.contains(&random_blob);
            let emitted = announce_emitted(random_blob);
            let timebase = existing_entry
                .random_blobs
                .iter()
                .copied()
                .map(announce_emitted)
                .max()
                .unwrap_or(0);
            let expired_for_replacement = now >= existing_entry.replacement_expires_at;
            let accepted = if hops > existing_entry.hops && expired_for_replacement {
                !seen
            } else {
                !seen && emitted > timebase
            };
            if !accepted {
                return None;
            }
            let mut blobs = existing_entry.random_blobs.clone();
            remember_random_blob(&mut blobs, random_blob);
            blobs
        } else {
            VecDeque::from([random_blob])
        };

        let new_entry = PathEntry {
            timestamp: now,
            observed_at,
            lifetime: path_lifetime(mode),
            replacement_expires_at: now + path_lifetime(mode),
            random_blobs,
            received_from,
            hops,
            iface,
            packet_hash: announce.hash(),
        };

        let was_active = self.map.insert(announce.destination, new_entry).is_some();

        log::info!(
            "{} is now reachable over {} hops through {} on iface {}",
            announce.destination,
            hops,
            received_from,
            iface,
        );
        if was_active {
            return None;
        }

        let kind = if let Some(index) = self
            .lost_destinations
            .iter()
            .position(|destination| *destination == announce.destination)
        {
            self.lost_destinations.remove(index);
            RouteEventKind::Rediscovered
        } else {
            RouteEventKind::Discovered
        };
        self.snapshot(&announce.destination, now).map(|route| RouteEvent {
            kind,
            route,
            loss_reason: None,
            occurred_at: observed_at,
        })
    }

    pub fn cull(
        &mut self,
        now: Instant,
        observed_at: SystemTime,
        active_interfaces: &[AddressHash],
    ) -> Vec<RouteEvent> {
        let mut removed = Vec::new();
        self.map.retain(|destination, entry| {
            let reason = if entry.is_expired(now) {
                Some(RouteLossReason::Expired)
            } else if !active_interfaces.contains(&entry.iface) {
                Some(RouteLossReason::InterfaceUnavailable)
            } else {
                None
            };
            if let Some(loss_reason) = reason {
                removed.push(RouteEvent {
                    kind: RouteEventKind::Lost,
                    route: entry.snapshot(*destination, now),
                    loss_reason: Some(loss_reason),
                    occurred_at: observed_at,
                });
                false
            } else {
                true
            }
        });
        for event in &removed {
            self.remember_lost(event.route.destination);
        }
        removed
    }

    fn remember_lost(&mut self, destination: AddressHash) {
        if self.lost_destinations.contains(&destination) {
            return;
        }
        if self.lost_destinations.len() >= LOST_DESTINATION_CAPACITY {
            self.lost_destinations.pop_front();
        }
        self.lost_destinations.push_back(destination);
    }

    pub fn handle_inbound_packet(
        &self,
        original_packet: &Packet,
        lookup: Option<AddressHash>,
    ) -> (Packet, Option<AddressHash>) {
        let lookup = lookup.unwrap_or(original_packet.destination);

        let entry = match self.map.get(&lookup) {
            Some(entry) => entry,
            None => return (*original_packet, None),
        };

        (
            Packet {
                header: Header {
                    ifac_flag: IfacFlag::Open, // IFAC applied at interface layer
                    header_type: HeaderType::Type2,
                    propagation_type: PropagationType::Transport,
                    hops: original_packet.header.hops,
                    ..original_packet.header
                },
                ifac: None,
                destination: original_packet.destination,
                transport: Some(entry.received_from),
                context: original_packet.context,
                data: original_packet.data,
            },
            Some(entry.iface),
        )
    }

    pub fn refresh(&mut self, destination: &AddressHash) {
        self.refresh_at(destination, Instant::now(), SystemTime::now());
    }

    fn refresh_at(&mut self, destination: &AddressHash, now: Instant, observed_at: SystemTime) {
        if let Some(entry) = self.map.get_mut(destination) {
            entry.timestamp = now;
            entry.observed_at = observed_at;
        }
    }

    pub fn handle_packet(&mut self, original_packet: &Packet) -> (Packet, Option<AddressHash>) {
        if original_packet.header.header_type == HeaderType::Type2 {
            return (*original_packet, None);
        }

        if original_packet.header.packet_type == PacketType::Announce {
            return (*original_packet, None);
        }

        if original_packet.header.destination_type == DestinationType::Plain
            || original_packet.header.destination_type == DestinationType::Group
        {
            return (*original_packet, None);
        }

        let entry = match self.map.get(&original_packet.destination) {
            Some(entry) => entry,
            None => return (*original_packet, None),
        };

        if entry.hops <= 1 {
            return (*original_packet, Some(entry.iface));
        }

        (
            Packet {
                header: Header {
                    header_type: HeaderType::Type2,
                    propagation_type: PropagationType::Transport,
                    ..original_packet.header
                },
                ifac: original_packet.ifac,
                destination: original_packet.destination,
                transport: Some(entry.received_from),
                context: original_packet.context,
                data: original_packet.data,
            },
            Some(entry.iface),
        )
    }
}

pub const fn path_lifetime(mode: InterfaceMode) -> Duration {
    match mode {
        InterfaceMode::AccessPoint => ACCESS_POINT_PATH_LIFETIME,
        InterfaceMode::Roaming => ROAMING_PATH_LIFETIME,
        _ => DEFAULT_PATH_LIFETIME,
    }
}

fn announce_emitted(blob: [u8; crate::destination::RAND_HASH_LENGTH]) -> u64 {
    u64::from_be_bytes([0, 0, 0, blob[5], blob[6], blob[7], blob[8], blob[9]])
}

fn remember_random_blob(
    blobs: &mut VecDeque<[u8; crate::destination::RAND_HASH_LENGTH]>,
    blob: [u8; crate::destination::RAND_HASH_LENGTH],
) {
    if blobs.len() >= RANDOM_BLOB_CAPACITY {
        blobs.pop_front();
    }
    blobs.push_back(blob);
}

impl Default for PathTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::StaticBuffer;
    use crate::packet::{ContextFlag, DestinationType, IfacFlag, PacketType, PropagationType};

    fn announce(destination: AddressHash, transport: AddressHash, hops: u8) -> Packet {
        Packet {
            header: Header {
                ifac_flag: IfacFlag::Open,
                header_type: HeaderType::Type2,
                context_flag: ContextFlag::Unset,
                propagation_type: PropagationType::Transport,
                destination_type: DestinationType::Single,
                packet_type: PacketType::Announce,
                hops,
            },
            ifac: None,
            destination,
            transport: Some(transport),
            context: crate::packet::PacketContext::None,
            data: StaticBuffer::new(),
        }
    }

    fn random_blob(emitted: u64) -> [u8; crate::destination::RAND_HASH_LENGTH] {
        let mut blob = [0u8; crate::destination::RAND_HASH_LENGTH];
        blob[5..].copy_from_slice(&emitted.to_be_bytes()[3..]);
        blob
    }

    #[test]
    fn handle_packet_direct_hop_preserves_type1_and_ifac_flag() {
        let destination = AddressHash::new_from_hash(&Hash::new_from_slice(b"destination"));
        let iface = AddressHash::new_from_hash(&Hash::new_from_slice(b"iface"));
        let mut table = PathTable::new();
        table.map.insert(
            destination,
            PathEntry {
                timestamp: Instant::now(),
                observed_at: SystemTime::now(),
                lifetime: DEFAULT_PATH_LIFETIME,
                replacement_expires_at: Instant::now() + DEFAULT_PATH_LIFETIME,
                random_blobs: VecDeque::from([random_blob(100)]),
                received_from: destination,
                hops: 1,
                iface,
                packet_hash: Hash::new_from_slice(b"packet"),
            },
        );

        let packet = Packet {
            header: Header {
                ifac_flag: IfacFlag::Open,
                header_type: HeaderType::Type1,
                context_flag: ContextFlag::Unset,
                propagation_type: PropagationType::Broadcast,
                destination_type: DestinationType::Single,
                packet_type: PacketType::Data,
                hops: 0,
            },
            ifac: None,
            destination,
            transport: None,
            context: crate::packet::PacketContext::None,
            data: StaticBuffer::new(),
        };

        let (forwarded, next_iface) = table.handle_packet(&packet);
        assert_eq!(next_iface, Some(iface));
        assert_eq!(forwarded.header.ifac_flag, IfacFlag::Open);
        assert_eq!(forwarded.header.header_type, HeaderType::Type1);
        assert_eq!(forwarded.transport, None);
    }

    #[test]
    fn handle_packet_multihop_promotes_to_type2_transport() {
        let destination = AddressHash::new_from_hash(&Hash::new_from_slice(b"destination"));
        let iface = AddressHash::new_from_hash(&Hash::new_from_slice(b"iface"));
        let next_hop = AddressHash::new_from_hash(&Hash::new_from_slice(b"next_hop"));
        let mut table = PathTable::new();
        table.map.insert(
            destination,
            PathEntry {
                timestamp: Instant::now(),
                observed_at: SystemTime::now(),
                lifetime: DEFAULT_PATH_LIFETIME,
                replacement_expires_at: Instant::now() + DEFAULT_PATH_LIFETIME,
                random_blobs: VecDeque::from([random_blob(100)]),
                received_from: next_hop,
                hops: 2,
                iface,
                packet_hash: Hash::new_from_slice(b"packet"),
            },
        );

        let packet = Packet {
            header: Header {
                ifac_flag: IfacFlag::Open,
                header_type: HeaderType::Type1,
                context_flag: ContextFlag::Unset,
                propagation_type: PropagationType::Broadcast,
                destination_type: DestinationType::Single,
                packet_type: PacketType::Data,
                hops: 0,
            },
            ifac: None,
            destination,
            transport: None,
            context: crate::packet::PacketContext::None,
            data: StaticBuffer::new(),
        };

        let (forwarded, next_iface) = table.handle_packet(&packet);
        assert_eq!(next_iface, Some(iface));
        assert_eq!(forwarded.header.ifac_flag, IfacFlag::Open);
        assert_eq!(forwarded.header.header_type, HeaderType::Type2);
        assert_eq!(forwarded.header.propagation_type, PropagationType::Transport);
        assert_eq!(forwarded.transport, Some(next_hop));
    }

    #[test]
    fn snapshot_preserves_route_identity_and_monotonic_age() {
        let now = Instant::now();
        let destination = AddressHash::new_from_hash(&Hash::new_from_slice(b"snapshot-dest"));
        let next_hop = AddressHash::new_from_hash(&Hash::new_from_slice(b"snapshot-hop"));
        let iface = AddressHash::new_from_hash(&Hash::new_from_slice(b"snapshot-iface"));
        let entry = PathEntry {
            timestamp: now - Duration::from_secs(7),
            observed_at: SystemTime::UNIX_EPOCH + Duration::from_secs(100),
            lifetime: DEFAULT_PATH_LIFETIME,
            replacement_expires_at: now + DEFAULT_PATH_LIFETIME,
            random_blobs: VecDeque::from([random_blob(100)]),
            received_from: next_hop,
            hops: 2,
            iface,
            packet_hash: Hash::new_from_slice(b"snapshot-packet"),
        };

        let snapshot = entry.snapshot(destination, now);

        assert_eq!(snapshot.destination, destination);
        assert_eq!(snapshot.received_from, next_hop);
        assert_eq!(snapshot.iface, iface);
        assert_eq!(snapshot.age, Duration::from_secs(7));
        assert_eq!(snapshot.observed_at, SystemTime::UNIX_EPOCH + Duration::from_secs(100));
    }

    #[test]
    fn newer_equal_cost_announce_refreshes_existing_route() {
        let destination = AddressHash::new_from_hash(&Hash::new_from_slice(b"refresh-dest"));
        let next_hop = AddressHash::new_from_hash(&Hash::new_from_slice(b"refresh-hop"));
        let iface = AddressHash::new_from_hash(&Hash::new_from_slice(b"refresh-iface"));
        let old_timestamp = Instant::now() - Duration::from_secs(10);
        let old_observed_at = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let mut table = PathTable::new();
        table.map.insert(
            destination,
            PathEntry {
                timestamp: old_timestamp,
                observed_at: old_observed_at,
                lifetime: DEFAULT_PATH_LIFETIME,
                replacement_expires_at: old_timestamp + DEFAULT_PATH_LIFETIME,
                random_blobs: VecDeque::from([random_blob(100)]),
                received_from: next_hop,
                hops: 2,
                iface,
                packet_hash: Hash::new_from_slice(b"old-packet"),
            },
        );

        table.handle_announce(
            &announce(destination, next_hop, 2),
            Some(next_hop),
            iface,
            InterfaceMode::Full,
            random_blob(101),
        );

        let refreshed = table.get(&destination).unwrap();
        assert!(refreshed.timestamp > old_timestamp);
        assert!(refreshed.observed_at > old_observed_at);
        assert_eq!(refreshed.received_from, next_hop);
        assert_eq!(refreshed.iface, iface);
        assert_eq!(refreshed.hops, 2);
    }

    #[test]
    fn replay_does_not_refresh_but_newer_equal_cost_route_replaces() {
        let started = Instant::now();
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let destination = AddressHash::new_from_hash(&Hash::new_from_slice(b"freshness-dest"));
        let first_hop = AddressHash::new_from_hash(&Hash::new_from_slice(b"first-hop"));
        let second_hop = AddressHash::new_from_hash(&Hash::new_from_slice(b"second-hop"));
        let first_iface = AddressHash::new_from_hash(&Hash::new_from_slice(b"first-iface"));
        let second_iface = AddressHash::new_from_hash(&Hash::new_from_slice(b"second-iface"));
        let packet = announce(destination, first_hop, 1);
        let mut table = PathTable::new();
        table.handle_announce_at(
            &packet,
            Some(first_hop),
            first_iface,
            InterfaceMode::Full,
            random_blob(100),
            (started, wall),
        );

        table.handle_announce_at(
            &packet,
            Some(first_hop),
            first_iface,
            InterfaceMode::Full,
            random_blob(100),
            (started + Duration::from_secs(10), wall + Duration::from_secs(10)),
        );
        let replayed = table.snapshot(&destination, started + Duration::from_secs(10)).unwrap();
        assert_eq!(replayed.age, Duration::from_secs(10));
        assert_eq!(replayed.received_from, first_hop);

        table.handle_announce_at(
            &packet,
            Some(second_hop),
            second_iface,
            InterfaceMode::Full,
            random_blob(101),
            (started + Duration::from_secs(20), wall + Duration::from_secs(20)),
        );
        let replaced = table.snapshot(&destination, started + Duration::from_secs(20)).unwrap();
        assert_eq!(replaced.age, Duration::ZERO);
        assert_eq!(replaced.received_from, second_hop);
        assert_eq!(replaced.iface, second_iface);
    }

    #[test]
    fn higher_hop_unseen_announce_can_replace_at_exact_expiry() {
        let started = Instant::now();
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let destination = AddressHash::new_from_hash(&Hash::new_from_slice(b"replace-dest"));
        let first_hop = AddressHash::new_from_hash(&Hash::new_from_slice(b"replace-first"));
        let second_hop = AddressHash::new_from_hash(&Hash::new_from_slice(b"replace-second"));
        let iface = AddressHash::new_from_hash(&Hash::new_from_slice(b"replace-iface"));
        let mut table = PathTable::new();
        table.handle_announce_at(
            &announce(destination, first_hop, 2),
            Some(first_hop),
            iface,
            InterfaceMode::Roaming,
            random_blob(100),
            (started, wall),
        );

        table.refresh_at(
            &destination,
            started + ROAMING_PATH_LIFETIME / 2,
            wall + ROAMING_PATH_LIFETIME / 2,
        );

        table.handle_announce_at(
            &announce(destination, second_hop, 3),
            Some(second_hop),
            iface,
            InterfaceMode::Roaming,
            random_blob(99),
            (started + ROAMING_PATH_LIFETIME, wall + ROAMING_PATH_LIFETIME),
        );

        let replaced = table.snapshot(&destination, started + ROAMING_PATH_LIFETIME).unwrap();
        assert_eq!(replaced.received_from, second_hop);
        assert_eq!(replaced.hops, 3);
        assert_eq!(replaced.age, Duration::ZERO);
    }

    #[test]
    fn route_lifetimes_match_canonical_interface_modes() {
        assert_eq!(path_lifetime(InterfaceMode::Full), Duration::from_secs(7 * 24 * 60 * 60));
        assert_eq!(path_lifetime(InterfaceMode::AccessPoint), Duration::from_secs(24 * 60 * 60));
        assert_eq!(path_lifetime(InterfaceMode::Roaming), Duration::from_secs(6 * 60 * 60));
        assert_eq!(path_lifetime(InterfaceMode::Boundary), DEFAULT_PATH_LIFETIME);
    }

    #[test]
    fn cull_keeps_route_at_deadline_and_removes_it_afterwards() {
        let started = Instant::now();
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let destination = AddressHash::new_from_hash(&Hash::new_from_slice(b"expiry-dest"));
        let next_hop = AddressHash::new_from_hash(&Hash::new_from_slice(b"expiry-hop"));
        let iface = AddressHash::new_from_hash(&Hash::new_from_slice(b"expiry-iface"));
        let mut table = PathTable::new();
        let discovered = table.handle_announce_at(
            &announce(destination, next_hop, 1),
            Some(next_hop),
            iface,
            InterfaceMode::Roaming,
            random_blob(100),
            (started, wall),
        );
        assert_eq!(discovered.map(|event| event.kind), Some(RouteEventKind::Discovered));

        assert!(
            table
                .cull(started + ROAMING_PATH_LIFETIME, wall + ROAMING_PATH_LIFETIME, &[iface])
                .is_empty()
        );
        assert!(table.get(&destination).is_some());

        let events = table.cull(
            started + ROAMING_PATH_LIFETIME + Duration::from_nanos(1),
            wall + ROAMING_PATH_LIFETIME,
            &[iface],
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, RouteEventKind::Lost);
        assert_eq!(events[0].loss_reason, Some(RouteLossReason::Expired));
        assert!(table.get(&destination).is_none());
        assert!(table.cull(started + DEFAULT_PATH_LIFETIME, wall, &[iface]).is_empty());
    }

    #[test]
    fn interface_loss_removes_route_and_next_announce_is_rediscovery() {
        let started = Instant::now();
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let destination = AddressHash::new_from_hash(&Hash::new_from_slice(b"loss-dest"));
        let next_hop = AddressHash::new_from_hash(&Hash::new_from_slice(b"loss-hop"));
        let iface = AddressHash::new_from_hash(&Hash::new_from_slice(b"loss-iface"));
        let packet = announce(destination, next_hop, 1);
        let mut table = PathTable::new();
        table.handle_announce_at(
            &packet,
            Some(next_hop),
            iface,
            InterfaceMode::Full,
            random_blob(100),
            (started, wall),
        );

        let events = table.cull(started + Duration::from_secs(1), wall, &[]);
        assert_eq!(events[0].loss_reason, Some(RouteLossReason::InterfaceUnavailable));
        assert_eq!(table.next_hop(&destination), None);

        let rediscovered = table.handle_announce_at(
            &packet,
            Some(next_hop),
            iface,
            InterfaceMode::Full,
            random_blob(100),
            (started + Duration::from_secs(2), wall + Duration::from_secs(2)),
        );
        assert_eq!(rediscovered.map(|event| event.kind), Some(RouteEventKind::Rediscovered));
        assert_eq!(table.next_hop(&destination), Some(next_hop));
    }

    #[test]
    fn explicit_routed_use_refreshes_route_but_direct_lookup_does_not() {
        let started = Instant::now();
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let destination = AddressHash::new_from_hash(&Hash::new_from_slice(b"usage-dest"));
        let next_hop = AddressHash::new_from_hash(&Hash::new_from_slice(b"usage-hop"));
        let iface = AddressHash::new_from_hash(&Hash::new_from_slice(b"usage-iface"));
        let mut table = PathTable::new();
        let mut routed = announce(destination, next_hop, 1);
        table.handle_announce_at(
            &routed,
            Some(next_hop),
            iface,
            InterfaceMode::Full,
            random_blob(100),
            (started, wall),
        );
        routed.header.packet_type = PacketType::Data;
        routed.header.header_type = HeaderType::Type1;
        table.refresh_at(
            &destination,
            started + Duration::from_secs(10),
            wall + Duration::from_secs(10),
        );
        assert_eq!(
            table.snapshot(&destination, started + Duration::from_secs(12)).unwrap().age,
            Duration::from_secs(2)
        );

        table.map.get_mut(&destination).unwrap().hops = 1;
        table.handle_packet(&routed);
        assert_eq!(
            table.snapshot(&destination, started + Duration::from_secs(22)).unwrap().age,
            Duration::from_secs(12)
        );
    }
}
