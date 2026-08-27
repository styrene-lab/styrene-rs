use std::{
    cmp::min,
    collections::HashMap,
    time::{Duration, Instant},
};

use crate::{
    hash::Hash,
    packet::{Packet, PacketContext, PacketType},
};

pub struct PacketTrack {
    pub time: Instant,
    pub min_hops: u8,
}

pub struct PacketCache {
    map: HashMap<Hash, PacketTrack>,
    resource_proofs: HashMap<Hash, Packet>,
    remove_cache: Vec<Hash>,
}

impl PacketCache {
    pub fn new() -> Self {
        Self { map: HashMap::new(), resource_proofs: HashMap::new(), remove_cache: Vec::new() }
    }

    pub fn release(&mut self, duration: Duration) {
        for entry in &self.map {
            if entry.1.time.elapsed() > duration {
                self.remove_cache.push(*entry.0);
            }
        }

        for hash in &self.remove_cache {
            self.map.remove(hash);
            self.resource_proofs.remove(hash);
        }

        self.remove_cache.clear();
    }

    pub fn update(&mut self, packet: &Packet) -> bool {
        let hash = packet.hash();

        let mut is_new_packet = false;

        let track = self.map.get_mut(&hash);
        if let Some(track) = track {
            track.time = Instant::now();
            track.min_hops = min(packet.header.hops, track.min_hops);
        } else {
            is_new_packet = true;

            self.map
                .insert(hash, PacketTrack { time: Instant::now(), min_hops: packet.header.hops });
        }
        if packet.header.packet_type == PacketType::Proof
            && packet.context == PacketContext::ResourceProof
        {
            self.resource_proofs.insert(hash, *packet);
        }

        is_new_packet
    }

    pub fn get(&self, hash: &Hash) -> Option<Packet> {
        self.resource_proofs.get(hash).copied()
    }
}
