use alloc::collections::{BTreeMap, BTreeSet, VecDeque};

use rand_core::OsRng;
use tokio::time::{Duration, Instant};

use crate::destination::{DestinationName, PlainInputDestination};
use crate::hash::{ADDRESS_HASH_SIZE, AddressHash};
use crate::identity::EmptyIdentity;
use crate::packet::{
    ContextFlag, DestinationType, Header, HeaderType, IfacFlag, Packet, PacketContext,
    PacketDataBuffer, PacketType, PropagationType,
};

pub const MAX_PR_TAGS: usize = 16_000;
pub const MAX_PENDING_DISCOVERY_REQUESTS: usize = 32;
pub const PATH_REQUEST_GATE_TIMEOUT: Duration = Duration::from_secs(45);

pub fn create_path_request_destination() -> PlainInputDestination {
    PlainInputDestination::new(
        EmptyIdentity {},
        DestinationName::new("rnstransport", "path.request"),
    )
}

pub type TagBytes = Vec<u8>;
type DuplicateKey = (AddressHash, TagBytes);

pub fn create_random_tag() -> TagBytes {
    AddressHash::new_from_rand(OsRng).as_slice().into()
}

#[derive(Debug, PartialEq, Eq)]
pub struct PathRequest {
    pub destination: AddressHash,
    pub requesting_transport: Option<AddressHash>,
    pub tag_bytes: TagBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRequestDecodeError {
    MissingDestination,
    MissingTag,
    ExcessiveTag,
}

impl PathRequest {
    pub(super) fn decode(
        data: &[u8],
        transport_name: &str,
    ) -> Result<Self, PathRequestDecodeError> {
        if data.len() <= ADDRESS_HASH_SIZE {
            log::info!(
                "tp({}): ignoring malformed path request: no {}",
                transport_name,
                if data.len() < ADDRESS_HASH_SIZE { "destination" } else { "tag" }
            );
            return Err(if data.len() < ADDRESS_HASH_SIZE {
                PathRequestDecodeError::MissingDestination
            } else {
                PathRequestDecodeError::MissingTag
            });
        }

        let mut destination = [0_u8; ADDRESS_HASH_SIZE];
        destination.copy_from_slice(&data[..ADDRESS_HASH_SIZE]);
        let destination = AddressHash::new(destination);
        let mut requesting_transport = None;
        let mut tag_start = ADDRESS_HASH_SIZE;
        if data.len() > ADDRESS_HASH_SIZE * 2 {
            requesting_transport =
                Some(AddressHash::new_from_slice(&data[ADDRESS_HASH_SIZE..2 * ADDRESS_HASH_SIZE]));
            tag_start = ADDRESS_HASH_SIZE * 2;
        }
        if data.len() - tag_start > ADDRESS_HASH_SIZE {
            return Err(PathRequestDecodeError::ExcessiveTag);
        }

        Ok(Self { destination, requesting_transport, tag_bytes: data[tag_start..].into() })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryAction {
    StartDiscovery,
    Batched,
    IngressLimited,
    PendingQueueFull,
    InactiveInterface,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PathRequestSnapshot {
    pub replay_current: u64,
    pub replay_previous: u64,
    pub in_flight: u64,
    pub pending_capacity: u64,
    pub pending_depth: u64,
    pub pending_dropped: u64,
}

struct DiscoveryEntry {
    expires_at: Instant,
    waiters: BTreeSet<AddressHash>,
    tag: TagBytes,
    ingress_iface: AddressHash,
    pending: bool,
    sent_ifaces: BTreeSet<AddressHash>,
}

pub struct PathRequests {
    replay_current: BTreeSet<DuplicateKey>,
    replay_previous: BTreeSet<DuplicateKey>,
    name: String,
    transport_id: Option<AddressHash>,
    controlled_destination: PlainInputDestination,
    in_flight: BTreeMap<AddressHash, Instant>,
    discovery: BTreeMap<AddressHash, DiscoveryEntry>,
    pending: VecDeque<AddressHash>,
    pending_dropped: u64,
    discovery_timeout: Duration,
}

impl PathRequests {
    pub fn new(
        name: &str,
        transport_id: Option<AddressHash>,
        _announce_queue_len: usize,
        _announce_cap: usize,
        request_timeout_secs: u64,
    ) -> Self {
        Self {
            replay_current: BTreeSet::new(),
            replay_previous: BTreeSet::new(),
            name: name.into(),
            transport_id,
            controlled_destination: create_path_request_destination(),
            in_flight: BTreeMap::new(),
            discovery: BTreeMap::new(),
            pending: VecDeque::new(),
            pending_dropped: 0,
            discovery_timeout: Duration::from_secs(request_timeout_secs.max(1)),
        }
    }

    pub fn decode(&mut self, data: &[u8]) -> Result<Option<PathRequest>, PathRequestDecodeError> {
        let path_request = PathRequest::decode(data, &self.name)?;
        let key = (path_request.destination, path_request.tag_bytes.clone());
        if self.replay_current.contains(&key) || self.replay_previous.contains(&key) {
            log::info!(
                "tp({}): ignoring duplicate path request for destination {}",
                self.name,
                path_request.destination
            );
            return Ok(None);
        }

        self.replay_current.insert(key);
        if self.replay_current.len() > MAX_PR_TAGS {
            self.replay_previous = core::mem::take(&mut self.replay_current);
        }
        Ok(Some(path_request))
    }

    pub fn generate(&mut self, destination: &AddressHash, tag: Option<TagBytes>) -> Packet {
        let mut data = PacketDataBuffer::new_from_slice(destination.as_slice());
        if let Some(transport_id) = self.transport_id {
            data.safe_write(transport_id.as_slice());
        }
        data.safe_write(tag.unwrap_or_else(create_random_tag).as_slice());

        Packet {
            header: Header {
                ifac_flag: IfacFlag::Open,
                header_type: HeaderType::Type1,
                context_flag: ContextFlag::Unset,
                propagation_type: PropagationType::Broadcast,
                destination_type: DestinationType::Plain,
                packet_type: PacketType::Data,
                hops: 0,
            },
            ifac: None,
            destination: self.controlled_destination.desc.address_hash,
            transport: self.transport_id,
            context: PacketContext::None,
            data,
        }
    }

    pub fn register_discovery(
        &mut self,
        request: &PathRequest,
        iface: AddressHash,
        ingress_limited: bool,
        active_interfaces: &BTreeSet<AddressHash>,
    ) -> DiscoveryAction {
        self.register_discovery_at(
            request,
            iface,
            ingress_limited,
            active_interfaces,
            Instant::now(),
        )
    }

    fn register_discovery_at(
        &mut self,
        request: &PathRequest,
        iface: AddressHash,
        ingress_limited: bool,
        active_interfaces: &BTreeSet<AddressHash>,
        now: Instant,
    ) -> DiscoveryAction {
        self.prune_at(now, active_interfaces);
        if !active_interfaces.contains(&iface) {
            return DiscoveryAction::InactiveInterface;
        }
        if self.in_flight.contains_key(&request.destination)
            && let Some(entry) = self.discovery.get_mut(&request.destination)
        {
            if ingress_limited {
                return DiscoveryAction::IngressLimited;
            }
            entry.waiters.insert(iface);
            return DiscoveryAction::Batched;
        }
        if self.in_flight.contains_key(&request.destination) {
            if ingress_limited {
                return DiscoveryAction::IngressLimited;
            }
            self.discovery.insert(
                request.destination,
                DiscoveryEntry {
                    expires_at: now + self.discovery_timeout,
                    waiters: BTreeSet::from([iface]),
                    tag: request.tag_bytes.clone(),
                    ingress_iface: iface,
                    pending: false,
                    sent_ifaces: BTreeSet::new(),
                },
            );
            return DiscoveryAction::Batched;
        }
        self.in_flight.insert(request.destination, now + PATH_REQUEST_GATE_TIMEOUT);
        if ingress_limited {
            return DiscoveryAction::IngressLimited;
        }
        let replacing_pending = self.pending.iter().any(|queued| *queued == request.destination);
        if self.pending.len() >= MAX_PENDING_DISCOVERY_REQUESTS && !replacing_pending {
            self.pending_dropped = self.pending_dropped.saturating_add(1);
            let mut waiters = self
                .discovery
                .remove(&request.destination)
                .map(|entry| entry.waiters)
                .unwrap_or_default();
            waiters.insert(iface);
            self.discovery.insert(
                request.destination,
                DiscoveryEntry {
                    expires_at: now + self.discovery_timeout,
                    waiters,
                    tag: request.tag_bytes.clone(),
                    ingress_iface: iface,
                    pending: false,
                    sent_ifaces: BTreeSet::new(),
                },
            );
            return DiscoveryAction::PendingQueueFull;
        }

        let mut waiters = self
            .discovery
            .remove(&request.destination)
            .map(|entry| entry.waiters)
            .unwrap_or_default();
        waiters.insert(iface);
        self.pending.retain(|queued| *queued != request.destination);
        self.discovery.insert(
            request.destination,
            DiscoveryEntry {
                expires_at: now + self.discovery_timeout,
                waiters,
                tag: request.tag_bytes.clone(),
                ingress_iface: iface,
                pending: true,
                sent_ifaces: BTreeSet::new(),
            },
        );
        self.pending.push_back(request.destination);
        DiscoveryAction::StartDiscovery
    }

    pub fn pending_packet(&mut self, destination: &AddressHash) -> Option<Packet> {
        let entry = self.discovery.get(destination)?;
        if !entry.pending {
            return None;
        }
        let tag = entry.tag.clone();
        Some(self.generate(destination, Some(tag)))
    }

    pub fn pending_front(&self) -> Option<AddressHash> {
        self.pending.front().copied()
    }

    pub fn pending_targets(
        &self,
        destination: &AddressHash,
        active_interfaces: &BTreeSet<AddressHash>,
    ) -> Vec<AddressHash> {
        let Some(entry) = self.discovery.get(destination) else {
            return Vec::new();
        };
        active_interfaces
            .iter()
            .filter(|iface| **iface != entry.ingress_iface && !entry.sent_ifaces.contains(*iface))
            .copied()
            .collect()
    }

    pub fn mark_iface_dispatched(&mut self, destination: &AddressHash, iface: AddressHash) {
        if let Some(entry) = self.discovery.get_mut(destination) {
            entry.sent_ifaces.insert(iface);
        }
    }

    pub fn mark_dispatched(&mut self, destination: &AddressHash) {
        if let Some(entry) = self.discovery.get_mut(destination) {
            entry.pending = false;
        }
        self.pending.retain(|queued| queued != destination);
    }

    pub fn rotate_pending(&mut self, destination: &AddressHash) {
        if self.pending.front() == Some(destination)
            && let Some(destination) = self.pending.pop_front()
        {
            self.pending.push_back(destination);
        }
    }

    pub fn take_waiters(
        &mut self,
        destination: &AddressHash,
        active_interfaces: &BTreeSet<AddressHash>,
    ) -> Vec<AddressHash> {
        self.in_flight.remove(destination);
        self.pending.retain(|queued| queued != destination);
        let Some(mut entry) = self.discovery.remove(destination) else {
            return Vec::new();
        };
        entry.waiters.retain(|iface| active_interfaces.contains(iface));
        entry.waiters.into_iter().collect()
    }

    pub fn retain_interfaces(&mut self, active_interfaces: &BTreeSet<AddressHash>) {
        self.prune_at(Instant::now(), active_interfaces);
    }

    fn prune_at(&mut self, now: Instant, active_interfaces: &BTreeSet<AddressHash>) {
        self.in_flight.retain(|_, expires_at| *expires_at >= now);
        self.discovery.retain(|_, entry| {
            entry.waiters.retain(|iface| active_interfaces.contains(iface));
            entry.expires_at > now && !entry.waiters.is_empty()
        });
        self.pending.retain(|destination| self.discovery.contains_key(destination));
    }

    pub fn snapshot(&self) -> PathRequestSnapshot {
        PathRequestSnapshot {
            replay_current: self.replay_current.len() as u64,
            replay_previous: self.replay_previous.len() as u64,
            in_flight: self.in_flight.len() as u64,
            pending_capacity: MAX_PENDING_DISCOVERY_REQUESTS as u64,
            pending_depth: self.pending.len() as u64,
            pending_dropped: self.pending_dropped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(destination: AddressHash, tag: u32) -> PathRequest {
        PathRequest {
            destination,
            requesting_transport: None,
            tag_bytes: tag.to_be_bytes().to_vec(),
        }
    }

    fn encoded(testee: &mut PathRequests, destination: AddressHash, tag: u32) -> Packet {
        testee.generate(&destination, Some(tag.to_be_bytes().to_vec()))
    }

    #[test]
    fn path_request_roundtrip() {
        let mut testee = PathRequests::new("", None, 16, 16, 30);
        let destination = AddressHash::new([0x11; 16]);
        let encoded = testee.generate(&destination, None);
        assert_eq!(
            testee.decode(encoded.data.as_slice()).unwrap().unwrap().destination,
            destination
        );
    }

    #[test]
    fn replay_generations_rotate_on_crossing_and_age_out_after_two_rotations() {
        let mut testee = PathRequests::new("", None, 16, 16, 30);
        let destination = AddressHash::new([0x22; 16]);
        for tag in 0..MAX_PR_TAGS as u32 {
            let packet = encoded(&mut testee, destination, tag);
            assert!(testee.decode(packet.data.as_slice()).unwrap().is_some());
        }
        assert_eq!(testee.snapshot().replay_current, MAX_PR_TAGS as u64);

        let crossing = encoded(&mut testee, destination, MAX_PR_TAGS as u32);
        assert!(testee.decode(crossing.data.as_slice()).unwrap().is_some());
        assert_eq!(testee.snapshot().replay_current, 0);
        assert_eq!(testee.snapshot().replay_previous, (MAX_PR_TAGS + 1) as u64);
        assert!(testee.decode(crossing.data.as_slice()).unwrap().is_none());

        for tag in (MAX_PR_TAGS + 1) as u32..=(MAX_PR_TAGS * 2 + 1) as u32 {
            let packet = encoded(&mut testee, destination, tag);
            assert!(testee.decode(packet.data.as_slice()).unwrap().is_some());
        }
        let oldest = encoded(&mut testee, destination, 0);
        assert!(testee.decode(oldest.data.as_slice()).unwrap().is_some());
    }

    #[test]
    fn destination_gate_batches_unique_active_waiters_and_rejects_limited_duplicates() {
        let mut testee = PathRequests::new("", None, 16, 16, 30);
        let destination = AddressHash::new([0x33; 16]);
        let iface_a = AddressHash::new([0xA1; 16]);
        let iface_b = AddressHash::new([0xB2; 16]);
        let active = BTreeSet::from([iface_a, iface_b]);
        let first = request(destination, 1);
        let second = request(destination, 2);

        assert_eq!(
            testee.register_discovery(&first, iface_a, false, &active),
            DiscoveryAction::StartDiscovery
        );
        assert_eq!(
            testee.register_discovery(&second, iface_a, false, &active),
            DiscoveryAction::Batched
        );
        assert_eq!(
            testee.register_discovery(&second, iface_b, true, &active),
            DiscoveryAction::IngressLimited
        );
        assert_eq!(testee.take_waiters(&destination, &active), vec![iface_a]);
    }

    #[test]
    fn gate_expires_after_45_seconds_and_has_no_global_cardinality_cap() {
        let mut testee = PathRequests::new("", None, 0, 0, 300);
        let now = Instant::now();
        let mut active = BTreeSet::new();
        for value in 0..64_u8 {
            active.insert(AddressHash::new([value; 16]));
        }
        let destination = AddressHash::new([0x44; 16]);
        let iface = AddressHash::new([0x01; 16]);
        assert_eq!(
            testee.register_discovery_at(&request(destination, 1), iface, false, &active, now),
            DiscoveryAction::StartDiscovery
        );
        testee.mark_dispatched(&destination);
        assert_eq!(
            testee.register_discovery_at(
                &request(destination, 2),
                iface,
                false,
                &active,
                now + PATH_REQUEST_GATE_TIMEOUT - Duration::from_nanos(1),
            ),
            DiscoveryAction::Batched
        );
        assert_eq!(
            testee.register_discovery_at(
                &request(destination, 3),
                iface,
                false,
                &active,
                now + PATH_REQUEST_GATE_TIMEOUT,
            ),
            DiscoveryAction::Batched
        );
        assert_eq!(
            testee.register_discovery_at(
                &request(destination, 4),
                iface,
                false,
                &active,
                now + PATH_REQUEST_GATE_TIMEOUT + Duration::from_nanos(1),
            ),
            DiscoveryAction::StartDiscovery
        );
        testee.mark_dispatched(&destination);
        for value in 2..64_u8 {
            let destination = AddressHash::new([value; 16]);
            assert_eq!(
                testee.register_discovery_at(
                    &request(destination, value as u32 + 4),
                    iface,
                    false,
                    &active,
                    now + PATH_REQUEST_GATE_TIMEOUT + Duration::from_nanos(1),
                ),
                DiscoveryAction::StartDiscovery
            );
            testee.mark_dispatched(&destination);
        }
        assert_eq!(testee.snapshot().in_flight, 63);
    }

    #[test]
    fn request_after_waiter_timeout_reattaches_without_restarting_discovery() {
        let mut testee = PathRequests::new("", None, 0, 0, 30);
        let now = Instant::now();
        let destination = AddressHash::new([0x45; 16]);
        let iface = AddressHash::new([0x01; 16]);
        let active = BTreeSet::from([iface]);
        assert_eq!(
            testee.register_discovery_at(&request(destination, 1), iface, false, &active, now),
            DiscoveryAction::StartDiscovery
        );
        testee.prune_at(now + Duration::from_secs(30), &active);
        assert_eq!(testee.snapshot().in_flight, 1);
        assert_eq!(
            testee.register_discovery_at(
                &request(destination, 2),
                iface,
                true,
                &active,
                now + Duration::from_secs(30),
            ),
            DiscoveryAction::IngressLimited
        );
        assert_eq!(
            testee.register_discovery_at(
                &request(destination, 3),
                iface,
                false,
                &active,
                now + Duration::from_secs(30),
            ),
            DiscoveryAction::Batched
        );
        assert_eq!(testee.take_waiters(&destination, &active), vec![iface]);
        assert_eq!(testee.snapshot().in_flight, 0);
        assert_eq!(
            testee.register_discovery_at(
                &request(destination, 4),
                iface,
                false,
                &active,
                now + PATH_REQUEST_GATE_TIMEOUT,
            ),
            DiscoveryAction::StartDiscovery
        );
    }

    #[test]
    fn local_answer_after_waiter_timeout_releases_destination_gate() {
        let mut testee = PathRequests::new("", None, 0, 0, 30);
        let now = Instant::now();
        let destination = AddressHash::new([0x46; 16]);
        let iface = AddressHash::new([0x01; 16]);
        let active = BTreeSet::from([iface]);
        assert_eq!(
            testee.register_discovery_at(&request(destination, 1), iface, false, &active, now),
            DiscoveryAction::StartDiscovery
        );

        testee.prune_at(now + Duration::from_secs(30), &active);
        assert!(testee.take_waiters(&destination, &active).is_empty());
        assert_eq!(testee.snapshot().in_flight, 0);
        assert_eq!(
            testee.register_discovery_at(
                &request(destination, 2),
                iface,
                false,
                &active,
                now + Duration::from_secs(30),
            ),
            DiscoveryAction::StartDiscovery
        );
    }

    #[test]
    fn pending_queue_accepts_32_and_observably_drops_33rd() {
        let mut testee = PathRequests::new("", None, 0, 0, 30);
        let iface = AddressHash::new([0x55; 16]);
        let active = BTreeSet::from([iface]);
        for value in 0..MAX_PENDING_DISCOVERY_REQUESTS as u8 {
            assert_eq!(
                testee.register_discovery(
                    &request(AddressHash::new([value; 16]), value as u32),
                    iface,
                    false,
                    &active,
                ),
                DiscoveryAction::StartDiscovery
            );
        }
        assert_eq!(
            testee.register_discovery(
                &request(AddressHash::new([0xFE; 16]), 99),
                iface,
                false,
                &active,
            ),
            DiscoveryAction::PendingQueueFull
        );
        let snapshot = testee.snapshot();
        assert_eq!(snapshot.pending_depth, 32);
        assert_eq!(snapshot.pending_dropped, 1);
        assert_eq!(snapshot.in_flight, 33);
        assert_eq!(
            testee.register_discovery(
                &request(AddressHash::new([0xFE; 16]), 100),
                iface,
                false,
                &active,
            ),
            DiscoveryAction::Batched
        );
        assert_eq!(testee.snapshot().pending_dropped, 1);
        assert_eq!(testee.take_waiters(&AddressHash::new([0xFE; 16]), &active), vec![iface]);
    }

    #[test]
    fn first_ingress_limited_request_gates_unrestricted_duplicate() {
        let mut testee = PathRequests::new("", None, 0, 0, 30);
        let destination = AddressHash::new([0xEF; 16]);
        let iface = AddressHash::new([0x55; 16]);
        let active = BTreeSet::from([iface]);

        assert_eq!(
            testee.register_discovery(&request(destination, 1), iface, true, &active),
            DiscoveryAction::IngressLimited
        );
        assert_eq!(testee.snapshot().in_flight, 1);
        assert_eq!(
            testee.register_discovery(&request(destination, 2), iface, false, &active),
            DiscoveryAction::Batched
        );
        assert_eq!(testee.snapshot().pending_depth, 0);
        assert_eq!(testee.take_waiters(&destination, &active), vec![iface]);
    }

    #[test]
    fn gate_restart_reuses_its_pending_slot_when_queue_is_full() {
        let mut testee = PathRequests::new("", None, 0, 0, 300);
        let now = Instant::now();
        let iface = AddressHash::new([0x55; 16]);
        let active = BTreeSet::from([iface]);
        for value in 0..MAX_PENDING_DISCOVERY_REQUESTS as u8 {
            assert_eq!(
                testee.register_discovery_at(
                    &request(AddressHash::new([value; 16]), value as u32),
                    iface,
                    false,
                    &active,
                    now,
                ),
                DiscoveryAction::StartDiscovery
            );
        }
        let destination = AddressHash::new([0; 16]);
        assert_eq!(
            testee.register_discovery_at(
                &request(destination, 99),
                iface,
                false,
                &active,
                now + PATH_REQUEST_GATE_TIMEOUT + Duration::from_nanos(1),
            ),
            DiscoveryAction::StartDiscovery
        );
        assert_eq!(testee.snapshot().pending_depth, MAX_PENDING_DISCOVERY_REQUESTS as u64);
        assert_eq!(testee.snapshot().pending_dropped, 0);
    }

    #[test]
    fn detached_waiters_are_filtered_before_response_and_timeout_cleans_pending_state() {
        let mut testee = PathRequests::new("", None, 0, 0, 30);
        let destination = AddressHash::new([0x66; 16]);
        let iface_a = AddressHash::new([0xA1; 16]);
        let iface_b = AddressHash::new([0xB2; 16]);
        let active = BTreeSet::from([iface_a, iface_b]);
        testee.register_discovery(&request(destination, 1), iface_a, false, &active);
        testee.register_discovery(&request(destination, 2), iface_b, false, &active);

        assert_eq!(testee.take_waiters(&destination, &BTreeSet::from([iface_b])), vec![iface_b]);
        assert_eq!(testee.snapshot().in_flight, 0);
        assert_eq!(testee.snapshot().pending_depth, 0);
    }

    #[test]
    fn excessive_tag_is_rejected_without_replay_state_mutation() {
        let mut testee = PathRequests::new("", None, 16, 16, 30);
        let destination = AddressHash::new([0x77; 16]);
        let mut data = destination.as_slice().to_vec();
        data.extend_from_slice(AddressHash::new([0x88; 16]).as_slice());
        data.extend_from_slice(&[0x55; ADDRESS_HASH_SIZE + 1]);
        assert_eq!(testee.decode(&data), Err(PathRequestDecodeError::ExcessiveTag));
        assert_eq!(testee.snapshot().replay_current, 0);
        assert_eq!(testee.snapshot().replay_previous, 0);
    }
}
