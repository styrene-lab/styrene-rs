use std::collections::HashMap;
use tokio::time::{Duration, Instant};

use crate::hash::AddressHash;
use crate::packet::{Header, HeaderType, IfacFlag, Packet};
use crate::transport::destination_ext::link::LinkId;

#[allow(dead_code)]
pub struct LinkEntry {
    pub timestamp: Instant,
    pub proof_timeout: Instant,
    pub next_hop: AddressHash,
    pub next_hop_iface: AddressHash,
    pub received_from: AddressHash,
    pub original_destination: AddressHash,
    pub taken_hops: u8,
    pub remaining_hops: u8,
    pub validated: bool,
}

fn send_backwards(packet: &Packet, entry: &LinkEntry) -> (Packet, AddressHash) {
    let propagated = Packet {
        header: Header {
            ifac_flag: IfacFlag::Open, // IFAC is applied at the interface layer, not transport
            header_type: HeaderType::Type2,
            hops: packet.header.hops,
            ..packet.header
        },
        ifac: None,
        destination: packet.destination,
        transport: Some(entry.next_hop),
        context: packet.context,
        data: packet.data,
    };

    (propagated, entry.received_from)
}

pub struct LinkTable {
    entries: HashMap<LinkId, LinkEntry>,
    fixed_proof_timeout: Option<Duration>,
    proof_timeout_per_hop: Duration,
    idle_timeout: Duration,
}

impl LinkTable {
    pub fn new(
        fixed_proof_timeout: Option<Duration>,
        proof_timeout_per_hop: Duration,
        idle_timeout: Duration,
    ) -> Self {
        Self { entries: HashMap::new(), fixed_proof_timeout, proof_timeout_per_hop, idle_timeout }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        link_request: &Packet,
        destination: AddressHash,
        received_from: AddressHash,
        next_hop: AddressHash,
        iface: AddressHash,
        remaining_hops: u8,
        outbound_bitrate: Option<u64>,
    ) {
        self.add_at(
            link_request,
            destination,
            received_from,
            next_hop,
            iface,
            remaining_hops,
            outbound_bitrate,
            Instant::now(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn add_at(
        &mut self,
        link_request: &Packet,
        destination: AddressHash,
        received_from: AddressHash,
        next_hop: AddressHash,
        iface: AddressHash,
        remaining_hops: u8,
        outbound_bitrate: Option<u64>,
        now: Instant,
    ) {
        let link_id = LinkId::from(link_request);

        let taken_hops = link_request.header.hops;
        let proof_timeout = super::deadlines::link_proof_timeout(
            self.fixed_proof_timeout,
            self.proof_timeout_per_hop,
            remaining_hops,
            outbound_bitrate,
        );

        let entry = LinkEntry {
            timestamp: now,
            proof_timeout: super::deadlines::deadline(now, proof_timeout),
            next_hop,
            next_hop_iface: iface,
            received_from,
            original_destination: destination,
            taken_hops,
            remaining_hops,
            validated: false,
        };

        self.entries.insert(link_id, entry);
    }

    pub fn original_destination(&self, link_id: &LinkId) -> Option<AddressHash> {
        self.entries.get(link_id).filter(|e| e.validated).map(|e| e.original_destination)
    }

    pub fn proof_validation_context(&self, link_id: &LinkId) -> Option<(AddressHash, AddressHash)> {
        self.entries.get(link_id).map(|entry| (entry.original_destination, entry.next_hop_iface))
    }

    pub fn handle_keepalive(&mut self, packet: &Packet) -> Option<(Packet, AddressHash)> {
        if let Some(entry) = self.entries.get_mut(&packet.destination) {
            entry.timestamp = Instant::now();
            return Some(send_backwards(packet, entry));
        }
        None
    }

    pub fn handle_proof(&mut self, proof: &Packet) -> Option<(Packet, AddressHash)> {
        match self.entries.get_mut(&proof.destination) {
            Some(entry) => {
                entry.remaining_hops = proof.header.hops;
                entry.validated = true;
                entry.timestamp = Instant::now();

                Some(send_backwards(proof, entry))
            }
            None => None,
        }
    }

    pub fn remove_stale(&mut self) {
        self.remove_stale_at(Instant::now());
    }

    fn remove_stale_at(&mut self, now: Instant) {
        let mut stale = vec![];

        for (link_id, entry) in &self.entries {
            if entry.validated {
                if super::deadlines::deadline(entry.timestamp, self.idle_timeout) <= now {
                    stale.push(*link_id);
                }
            } else if entry.proof_timeout < now {
                stale.push(*link_id);
            }
        }

        for link_id in stale {
            self.entries.remove(&link_id);
        }
    }

    pub fn remove_unavailable_interfaces(&mut self, active_interfaces: &[AddressHash]) {
        self.entries.retain(|_, entry| {
            active_interfaces.contains(&entry.received_from)
                && active_interfaces.contains(&entry.next_hop_iface)
        });
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(crate) fn proof_timeout_for_test(&self, link_id: &LinkId) -> Option<Instant> {
        self.entries.get(link_id).map(|entry| entry.proof_timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::Hash;

    fn address(value: &[u8]) -> AddressHash {
        AddressHash::new_from_hash(&Hash::new_from_slice(value))
    }

    #[test]
    fn unavailable_ingress_or_egress_removes_intermediate_link() {
        let ingress = address(b"ingress");
        let egress = address(b"egress");
        let link_id = address(b"link");
        let now = Instant::now();
        let mut table = LinkTable::new(None, Duration::from_secs(10), Duration::from_secs(10));
        table.entries.insert(
            link_id,
            LinkEntry {
                timestamp: now,
                proof_timeout: now + Duration::from_secs(10),
                next_hop: address(b"hop"),
                next_hop_iface: egress,
                received_from: ingress,
                original_destination: address(b"destination"),
                taken_hops: 1,
                remaining_hops: 1,
                validated: true,
            },
        );

        table.remove_unavailable_interfaces(&[ingress, egress]);
        assert_eq!(table.entries.len(), 1);

        table.remove_unavailable_interfaces(&[egress]);
        assert!(table.entries.is_empty());
    }

    #[test]
    fn proof_deadline_uses_remaining_hops_and_outbound_bitrate() {
        let ingress = address(b"ingress");
        let egress = address(b"egress");
        let destination = address(b"destination");
        let next_hop = address(b"hop");
        let packet = Packet { destination, ..Default::default() };
        let link_id = LinkId::from(&packet);
        let now = Instant::now();
        let mut table = LinkTable::new(None, Duration::from_secs(6), Duration::from_secs(10));

        table.add_at(&packet, destination, ingress, next_hop, egress, 3, Some(500), now);
        assert_eq!(table.entries[&link_id].proof_timeout, now + Duration::from_secs(26));
        table.remove_stale_at(now + Duration::from_secs(26));
        assert!(table.entries.contains_key(&link_id));
        table.remove_stale_at(now + Duration::from_secs(26) + Duration::from_nanos(1));
        assert!(!table.entries.contains_key(&link_id));
    }

    #[test]
    fn repeated_link_request_replaces_route_and_deadline_inputs() {
        let destination = address(b"destination");
        let packet = Packet { destination, ..Default::default() };
        let link_id = LinkId::from(&packet);
        let now = Instant::now();
        let mut table = LinkTable::new(None, Duration::from_secs(6), Duration::from_secs(10));

        table.add_at(
            &packet,
            destination,
            address(b"ingress-a"),
            address(b"hop-a"),
            address(b"egress-a"),
            3,
            Some(500),
            now,
        );
        table.add_at(
            &packet,
            destination,
            address(b"ingress-b"),
            address(b"hop-b"),
            address(b"egress-b"),
            1,
            Some(1_000),
            now + Duration::from_secs(1),
        );

        let entry = &table.entries[&link_id];
        assert_eq!(entry.received_from, address(b"ingress-b"));
        assert_eq!(entry.next_hop_iface, address(b"egress-b"));
        assert_eq!(entry.remaining_hops, 1);
        assert_eq!(entry.proof_timeout, now + Duration::from_secs(11));
    }

    #[test]
    fn extreme_configured_timeout_cannot_overflow_absolute_deadline() {
        let destination = address(b"destination");
        let packet = Packet { destination, ..Default::default() };
        let link_id = LinkId::from(&packet);
        let now = Instant::now();
        let mut table = LinkTable::new(Some(Duration::MAX), Duration::MAX, Duration::MAX);

        table.add_at(
            &packet,
            destination,
            address(b"ingress"),
            address(b"hop"),
            address(b"egress"),
            u8::MAX,
            Some(1),
            now,
        );
        assert!(
            table.entries[&link_id].proof_timeout
                > now + Duration::from_secs(2 * 365 * 24 * 60 * 60)
        );
    }
}
