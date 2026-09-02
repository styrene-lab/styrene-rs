#[cfg(test)]
mod tests {
    use super::*;
    use crate::destination::{DestinationDesc, DestinationName};
    use crate::identity::PrivateIdentity;
    use crate::transport::time::ManualMonotonicClock;
    use rand_core::OsRng;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn plain_resource_packets(
        link: &Link,
        data: &[u8],
        marker: u8,
    ) -> (Hash, Packet, Packet) {
        let random_hash = [marker; RANDOM_HASH_SIZE];
        let mut wire_data = vec![marker.wrapping_add(1); RANDOM_HASH_SIZE];
        wire_data.extend_from_slice(data);
        let hash = Hash::new(
            sha2::Sha256::new()
                .chain_update(data)
                .chain_update(random_hash)
                .finalize()
                .into(),
        );
        let advertisement = ResourceAdvertisement {
            transfer_size: wire_data.len() as u64,
            data_size: data.len() as u64,
            parts: 1,
            hash,
            random_hash,
            original_hash: hash,
            segment_index: 1,
            total_segments: 1,
            request_id: None,
            flags: 0,
            hashmap: map_hash(&wire_data, &random_hash).to_vec(),
        };
        (
            hash,
            resource_packet(
                PacketContext::ResourceAdvrtisement,
                &advertisement.pack().expect("advertisement"),
                *link.id(),
            ),
            resource_packet(PacketContext::Resource, &wire_data, *link.id()),
        )
    }

    fn completed_resource(
        flags: u8,
        request_id: Option<[u8; ADDRESS_HASH_SIZE]>,
        handler: crate::destination::IngressHandler,
    ) -> (Vec<Packet>, Vec<ResourceEvent>, usize) {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource-ingress"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(4);
        let mut link = Link::new(destination, tx);
        let _ = link.prove();
        let data = b"verified resource ingress";
        let mut wire_data = vec![0x43; RANDOM_HASH_SIZE];
        wire_data.extend_from_slice(data);
        let random_hash = [0x44; RANDOM_HASH_SIZE];
        let hash = Hash::new(sha2::Sha256::new().chain_update(data).chain_update(random_hash).finalize().into());
        let advertisement = ResourceAdvertisement {
            transfer_size: wire_data.len() as u64,
            data_size: data.len() as u64,
            parts: 1,
            hash,
            random_hash,
            original_hash: hash,
            segment_index: 1,
            total_segments: 1,
            request_id: request_id.map(|id| ByteBuf::from(id.to_vec())),
            flags,
            hashmap: map_hash(&wire_data, &random_hash).to_vec(),
        };
        let context = crate::destination::IngressContext {
            destination: destination.address_hash,
            link_id: *link.id(),
            kind: crate::destination::IngressKind::UnsolicitedResource,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let wrapped: crate::destination::IngressHandler = Arc::new(move |data, context| {
            counted.fetch_add(1, Ordering::SeqCst);
            handler(data, context)
        });
        let mut manager = ResourceManager::new();
        let advertisement_packet = resource_packet(
            PacketContext::ResourceAdvrtisement,
            &advertisement.pack().unwrap(),
            *link.id(),
        );
        assert_eq!(
            manager
                .handle_packet_with_ingress(
                    &advertisement_packet,
                    &mut link,
                    Some((&wrapped, &context)),
                    None,
                )
                .len(),
            1
        );
        manager.drain_events();
        let part = resource_packet(PacketContext::Resource, &wire_data, *link.id());
        let responses = manager.handle_packet_with_ingress(
            &part,
            &mut link,
            Some((&wrapped, &context)),
            None,
        );
        (responses, manager.drain_events(), calls.load(Ordering::SeqCst))
    }

    #[test]
    fn authoritative_unsolicited_resource_ingress_precedes_proof_and_complete() {
        let accepted: crate::destination::IngressHandler = Arc::new(|data, context| {
            assert_eq!(data, b"verified resource ingress");
            assert_eq!(context.kind, crate::destination::IngressKind::UnsolicitedResource);
            true
        });
        let (responses, events, calls) = completed_resource(0, None, accepted);
        assert_eq!(calls, 1);
        assert!(responses.iter().any(|packet| packet.context == PacketContext::ResourceProof));
        assert!(matches!(events.as_slice(), [ResourceEvent { kind: ResourceEventKind::Complete(_), .. }]));

        for rejected in [
            Arc::new(|_: &[u8], _: &crate::destination::IngressContext| false)
                as crate::destination::IngressHandler,
            Arc::new(|_: &[u8], _: &crate::destination::IngressContext| -> bool {
                panic!("ingress panic")
            })
                as crate::destination::IngressHandler,
        ] {
            let (responses, events, calls) = completed_resource(0, None, rejected);
            assert_eq!(calls, 1);
            assert!(!responses.iter().any(|packet| packet.context == PacketContext::ResourceProof));
            assert!(matches!(events.as_slice(), [ResourceEvent { kind: ResourceEventKind::Failed(ResourceFailure::Cancelled), .. }]));
        }
    }

    #[test]
    fn receive_handler_panic_releases_receiver_and_allows_followup_completion() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource-handler-recovery"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        let _ = link.prove();
        let context = crate::destination::IngressContext {
            destination: destination.address_hash,
            link_id: *link.id(),
            kind: crate::destination::IngressKind::UnsolicitedResource,
        };
        let panicking: crate::destination::IngressHandler = Arc::new(|_, _| panic!("ingress panic"));
        let accepting: crate::destination::IngressHandler = Arc::new(|data, _| data == b"second");
        let mut manager = ResourceManager::new();

        let (failed_hash, advertisement, part) = plain_resource_packets(&link, b"first", 0x61);
        assert_eq!(
            manager
                .handle_packet_with_ingress(
                    &advertisement,
                    &mut link,
                    Some((&panicking, &context)),
                    None,
                )
                .len(),
            1
        );
        assert_eq!(
            manager
                .handle_packet_with_ingress(
                    &part,
                    &mut link,
                    Some((&panicking, &context)),
                    None,
                )
                .iter()
                .filter(|packet| packet.context == PacketContext::ResourceReceiverCancel)
                .count(),
            1
        );
        assert!(manager.incoming.is_empty());
        assert!(matches!(
            manager.drain_events().as_slice(),
            [ResourceEvent {
                hash,
                kind: ResourceEventKind::Failed(ResourceFailure::Cancelled),
                ..
            }] if *hash == failed_hash
        ));

        let (completed_hash, advertisement, part) =
            plain_resource_packets(&link, b"second", 0x71);
        assert_eq!(
            manager
                .handle_packet_with_ingress(
                    &advertisement,
                    &mut link,
                    Some((&accepting, &context)),
                    None,
                )
                .len(),
            1
        );
        assert_eq!(
            manager
                .handle_packet_with_ingress(
                    &part,
                    &mut link,
                    Some((&accepting, &context)),
                    None,
                )
                .iter()
                .filter(|packet| packet.context == PacketContext::ResourceProof)
                .count(),
            1
        );
        assert!(manager.incoming.is_empty());
        assert!(matches!(
            manager.drain_events().as_slice(),
            [ResourceEvent {
                hash,
                kind: ResourceEventKind::Complete(_),
                ..
            }] if *hash == completed_hash
        ));
        assert!(manager
            .handle_packet_with_ingress(
                &part,
                &mut link,
                Some((&accepting, &context)),
                None,
            )
            .is_empty());
        assert!(manager.drain_events().is_empty());
    }

    #[test]
    fn destination_limit_applies_to_request_resources_and_response_resources_bypass_it() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource-limit"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        let _ = link.prove();
        let advertisement = ResourceAdvertisement {
            transfer_size: 101,
            data_size: 101,
            parts: 1,
            hash: Hash::new([0x31; HASH_SIZE]),
            random_hash: [0x32; RANDOM_HASH_SIZE],
            original_hash: Hash::new([0x31; HASH_SIZE]),
            segment_index: 1,
            total_segments: 1,
            request_id: None,
            flags: 0,
            hashmap: vec![0x33; MAPHASH_LEN],
        };
        let mut manager = ResourceManager::new();

        let mut exact_advertisement = advertisement.clone();
        exact_advertisement.transfer_size = 100;
        exact_advertisement.data_size = 100;
        exact_advertisement.hash = Hash::new([0x35; HASH_SIZE]);
        exact_advertisement.original_hash = exact_advertisement.hash;
        exact_advertisement.flags = FLAG_REQUEST;
        exact_advertisement.request_id = Some(ByteBuf::from(vec![0x34; ADDRESS_HASH_SIZE]));
        let exact_packet = resource_packet(
            PacketContext::ResourceAdvrtisement,
            &exact_advertisement.pack().unwrap(),
            *link.id(),
        );
        assert_eq!(
            manager.handle_packet_with_ingress(&exact_packet, &mut link, None, Some(100)).len(),
            1
        );

        let mut request_advertisement = advertisement.clone();
        request_advertisement.flags = FLAG_REQUEST;
        request_advertisement.request_id = Some(ByteBuf::from(vec![0x34; ADDRESS_HASH_SIZE]));
        let request_packet = resource_packet(
            PacketContext::ResourceAdvrtisement,
            &request_advertisement.pack().unwrap(),
            *link.id(),
        );
        assert!(
            manager.handle_packet_with_ingress(&request_packet, &mut link, None, Some(100)).is_empty()
        );

        let mut response_advertisement = advertisement;
        response_advertisement.hash = Hash::new([0x36; HASH_SIZE]);
        response_advertisement.original_hash = response_advertisement.hash;
        response_advertisement.flags = FLAG_RESPONSE;
        response_advertisement.request_id = Some(ByteBuf::from(vec![0x37; ADDRESS_HASH_SIZE]));
        let response_packet = resource_packet(
            PacketContext::ResourceAdvrtisement,
            &response_advertisement.pack().unwrap(),
            *link.id(),
        );
        assert_eq!(
            manager.handle_packet_with_ingress(&response_packet, &mut link, None, Some(100)).len(),
            1
        );
    }

    #[test]
    fn request_and_response_resources_bypass_ingress_and_closed_links_cannot_complete() {
        let rejecting: crate::destination::IngressHandler = Arc::new(|_, _| false);
        for flags in [FLAG_REQUEST, FLAG_RESPONSE] {
            let (responses, events, calls) =
                completed_resource(flags, Some([0x55; ADDRESS_HASH_SIZE]), rejecting.clone());
            assert_eq!(calls, 0);
            assert!(responses.iter().any(|packet| packet.context == PacketContext::ResourceProof));
            assert!(matches!(events.as_slice(), [ResourceEvent { kind: ResourceEventKind::Complete(_), .. }]));
        }

        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "closed-resource"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        let _ = link.prove();
        link.close();
        let context = crate::destination::IngressContext {
            destination: destination.address_hash,
            link_id: *link.id(),
            kind: crate::destination::IngressKind::UnsolicitedResource,
        };
        let mut manager = ResourceManager::new();
        let packet = resource_packet(PacketContext::Resource, b"closed", *link.id());
        assert!(manager
            .handle_packet_with_ingress(&packet, &mut link, Some((&rejecting, &context)), None)
            .is_empty());
        assert!(manager.drain_events().is_empty());
    }

    #[test]
    fn resource_sender_rejects_oversized_metadata() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let link = Link::new(destination, tx);
        let data = vec![0u8; 4];
        let metadata = vec![0u8; METADATA_MAX_SIZE + 1];

        let result = ResourceSender::new(
            &link,
            data,
            Some(metadata),
            None,
            false,
            Duration::ZERO,
        );
        assert!(matches!(result, Err(RnsError::InvalidArgument)));
    }

    #[test]
    fn resource_sender_chunks_at_negotiated_resource_sdu() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource-mtu"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        link.set_request_mtu(Some(1024));

        let sender = ResourceSender::new(
            &link,
            vec![0x5a; 3000],
            None,
            None,
            false,
            Duration::ZERO,
        )
        .expect("negotiated-MTU resource");

        assert!(sender.parts.iter().all(|part| part.len() <= 988));
        assert!(sender.parts.iter().any(|part| part.len() > PACKET_MDU));
    }

    #[test]
    fn resource_response_advertisement_matches_python_flags_and_request_id() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let link = Link::new(destination, tx);
        let request_id = [0x42; ADDRESS_HASH_SIZE];
        let packed_response = crate::transport::request::encode_response_envelope(
            request_id,
            &[0xc4, 0x02, 0x51, 0x51],
        )
        .expect("packed response");
        let mut manager = ResourceManager::new();

        let (resource_hash, packet) = manager
            .start_response(&link, packed_response.clone(), request_id)
            .expect("response resource");
        let mut decrypt_buf = vec![0u8; packet.data.len()];
        let plaintext = link
            .decrypt(packet.data.as_slice(), &mut decrypt_buf)
            .expect("advertisement decrypt");
        let advertisement = ResourceAdvertisement::unpack(plaintext).expect("advertisement");

        assert_eq!(packet.context, PacketContext::ResourceAdvrtisement);
        assert!(advertisement.is_response());
        assert!(!advertisement.is_request());
        assert_eq!(advertisement.request_id.as_ref().map(|id| id.as_slice()), Some(request_id.as_slice()));
        assert_eq!(advertisement.flags & FLAG_RESPONSE, FLAG_RESPONSE);
        assert_eq!(advertisement.flags & FLAG_REQUEST, 0);
        assert_eq!(advertisement.data_size, packed_response.len() as u64);
        assert!(manager.pending_outgoing.contains_key(&resource_hash));

        manager.confirm_outbound_dispatch(resource_hash, true);
        assert!(!manager.pending_outgoing.contains_key(&resource_hash));
        assert!(manager.outgoing.contains_key(&resource_hash));
    }

    #[test]
    fn resource_request_advertisement_matches_canonical_request_id_and_flags() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("nomadnetwork", "request"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let link = Link::new(destination, tx);
        let packed_request = crate::transport::request::encode_request_envelope(
            1_700_000_000.25,
            [0x22; ADDRESS_HASH_SIZE],
            &[0xc0],
        )
        .expect("packed request");
        let request_id = crate::transport::request::canonical_request_id(&packed_request);
        let mut manager = ResourceManager::new();
        let (_, packet) = manager
            .start_request(&link, packed_request, request_id)
            .expect("request resource");
        let mut decrypt_buf = vec![0u8; packet.data.len()];
        let plaintext = link
            .decrypt(packet.data.as_slice(), &mut decrypt_buf)
            .expect("advertisement decrypt");
        let advertisement = ResourceAdvertisement::unpack(plaintext).expect("advertisement");

        assert!(advertisement.is_request());
        assert!(!advertisement.is_response());
        assert_eq!(
            advertisement.request_id.as_ref().map(|id| id.as_slice()),
            Some(request_id.as_slice())
        );
    }

    #[test]
    fn resource_manager_rejects_split_flag() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        link.request();

        let adv = ResourceAdvertisement {
            transfer_size: 1,
            data_size: 1,
            parts: 1,
            hash: Hash::new_from_slice(&[1, 2, 3, 4]),
            random_hash: [0u8; RANDOM_HASH_SIZE],
            original_hash: Hash::new_from_slice(&[1, 2, 3, 4]),
            segment_index: 1,
            total_segments: 1,
            request_id: None,
            flags: FLAG_SPLIT,
            hashmap: vec![0u8; MAPHASH_LEN],
        };

        let packet =
            resource_packet(PacketContext::ResourceAdvrtisement, &adv.pack().expect("advertisement"), *link.id());

        let mut manager = ResourceManager::new_with_config(Duration::from_secs(1), 1);
        let responses = manager.handle_packet(&packet, &mut link);

        assert!(responses.is_empty());
        assert!(manager.incoming.is_empty());
    }

    #[test]
    fn resource_manager_ignores_duplicate_active_advertisement() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        link.request();

        let part = b"hello-resource";
        let random_hash = [7u8; RANDOM_HASH_SIZE];
        let mut hashmap = Vec::with_capacity(MAPHASH_LEN);
        hashmap.extend_from_slice(&map_hash(part, &random_hash));
        let adv = ResourceAdvertisement {
            transfer_size: part.len() as u64,
            data_size: part.len() as u64,
            parts: 1,
            hash: Hash::new_from_slice(&[9u8; 32]),
            random_hash,
            original_hash: Hash::new_from_slice(&[9u8; 32]),
            segment_index: 1,
            total_segments: 1,
            request_id: None,
            flags: 0,
            hashmap,
        };

        let packet =
            resource_packet(PacketContext::ResourceAdvrtisement, &adv.pack().expect("advertisement"), *link.id());

        let mut manager = ResourceManager::new_with_config(Duration::from_secs(1), 1);
        let first = manager.handle_packet(&packet, &mut link);
        assert_eq!(first.len(), 1);
        assert_eq!(manager.incoming.len(), 1);
        assert_eq!(
            manager.incoming.get(&adv.hash).expect("receiver").retry_count,
            0,
            "the initial request is not a retry"
        );

        let second = manager.handle_packet(&packet, &mut link);
        assert!(second.is_empty());
        assert_eq!(manager.incoming.len(), 1);
        assert_eq!(
            manager.incoming.get(&adv.hash).expect("receiver").retry_count,
            0
        );
    }

    #[test]
    fn resource_manager_removes_failed_receiver_without_followup_request() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        link.request();

        let part = b"not-bzip";
        let random_hash = [5u8; RANDOM_HASH_SIZE];
        let resource_hash = Hash::new_from_slice(&[8u8; 32]);
        let mut hashmap = Vec::with_capacity(MAPHASH_LEN);
        hashmap.extend_from_slice(&map_hash(part, &random_hash));
        let adv = ResourceAdvertisement {
            transfer_size: part.len() as u64,
            data_size: part.len() as u64,
            parts: 1,
            hash: resource_hash,
            random_hash,
            original_hash: resource_hash,
            segment_index: 1,
            total_segments: 1,
            request_id: None,
            flags: FLAG_COMPRESSED,
            hashmap,
        };

        let adv_packet =
            resource_packet(PacketContext::ResourceAdvrtisement, &adv.pack().expect("advertisement"), *link.id());

        let mut manager = ResourceManager::new_with_config(Duration::from_secs(1), 1);
        let first = manager.handle_packet(&adv_packet, &mut link);
        assert_eq!(first.len(), 1);
        assert_eq!(manager.incoming.len(), 1);

        let part_packet = resource_packet(PacketContext::Resource, part, *link.id());
        let responses = manager.handle_packet(&part_packet, &mut link);
        assert!(responses.is_empty());
        assert!(manager.incoming.is_empty());
        assert!(matches!(
            manager.drain_events().as_slice(),
            [ResourceEvent { kind: ResourceEventKind::Failed(ResourceFailure::Integrity), .. }]
        ));
    }

    #[test]
    fn resource_receiver_rejects_unreasonable_advertised_parts() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        link.request();

        let adv = ResourceAdvertisement {
            transfer_size: 1,
            data_size: 1,
            parts: 2,
            hash: Hash::new_from_slice(&[3u8; 32]),
            random_hash: [0u8; RANDOM_HASH_SIZE],
            original_hash: Hash::new_from_slice(&[3u8; 32]),
            segment_index: 1,
            total_segments: 1,
            request_id: None,
            flags: 0,
            hashmap: vec![0u8; MAPHASH_LEN * 2],
        };

        let packet =
            resource_packet(PacketContext::ResourceAdvrtisement, &adv.pack().expect("advertisement"), *link.id());

        let mut manager = ResourceManager::new_with_config(Duration::from_secs(1), 1);
        let responses = manager.handle_packet(&packet, &mut link);

        assert!(responses.is_empty());
        assert!(manager.incoming.is_empty());
    }

    fn bounded_advertisement(transfer_size: usize, data_size: usize, resource_sdu: usize) -> ResourceAdvertisement {
        let parts = transfer_size.div_ceil(resource_sdu);
        ResourceAdvertisement {
            transfer_size: transfer_size as u64,
            data_size: data_size as u64,
            parts: parts as u32,
            hash: Hash::new_from_slice(&[0x31; 32]),
            random_hash: [0x32; RANDOM_HASH_SIZE],
            original_hash: Hash::new_from_slice(&[0x31; 32]),
            segment_index: 1,
            total_segments: parts.div_ceil(HASHMAP_MAX_LEN) as u32,
            request_id: None,
            flags: 0,
            hashmap: vec![0; parts.min(HASHMAP_MAX_LEN) * MAPHASH_LEN],
        }
    }

    #[test]
    fn receiver_rejects_huge_parts_overflow_and_size_before_allocation() {
        let resource_sdu = 383;
        let mut huge_parts = bounded_advertisement(1, 1, resource_sdu);
        huge_parts.parts = u32::MAX;
        assert!(ResourceReceiver::new(
            &huge_parts,
            AddressHash::new([0; 16]),
            resource_sdu,
            MAX_UNSOLICITED_RESOURCE_SIZE,
            Duration::ZERO,
        )
        .is_err());

        let mut overflow = bounded_advertisement(resource_sdu, resource_sdu, resource_sdu);
        overflow.segment_index = u32::MAX;
        overflow.total_segments = u32::MAX;
        assert!(ResourceReceiver::new(
            &overflow,
            AddressHash::new([0; 16]),
            resource_sdu,
            MAX_UNSOLICITED_RESOURCE_SIZE,
            Duration::ZERO,
        )
        .is_err());

        let oversized = bounded_advertisement(
            MAX_UNSOLICITED_RESOURCE_SIZE + 1,
            MAX_UNSOLICITED_RESOURCE_SIZE + 1,
            resource_sdu,
        );
        assert!(ResourceReceiver::new(
            &oversized,
            AddressHash::new([0; 16]),
            resource_sdu,
            MAX_UNSOLICITED_RESOURCE_SIZE,
            Duration::ZERO,
        )
        .is_err());
    }

    #[test]
    fn receiver_accepts_exact_unsolicited_and_negotiated_nomadnet_limits() {
        let resource_sdu = 383;
        for limit in [MAX_UNSOLICITED_RESOURCE_SIZE, MAX_NEGOTIATED_RESOURCE_SIZE] {
            let advertisement = bounded_advertisement(limit, limit, resource_sdu);
            let receiver = ResourceReceiver::new(
                &advertisement,
                AddressHash::new([0; 16]),
                resource_sdu,
                limit,
                Duration::ZERO,
            )
            .expect("exact resource limit");
            assert_eq!(receiver.parts.len(), limit.div_ceil(resource_sdu));
        }
    }

    #[test]
    fn bounded_decompression_rejects_bomb_and_accepts_exact_declared_output() {
        use bzip2::write::BzEncoder;
        use bzip2::Compression;
        use std::io::Write as _;

        let payload = vec![0x41; 1024 * 1024];
        let mut encoder = BzEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(&payload).unwrap();
        let compressed = encoder.finish().unwrap();
        assert_eq!(
            decompress_payload_bounded(&compressed, payload.len(), payload.len()).unwrap(),
            payload
        );
        assert!(decompress_payload_bounded(&compressed, 1024, 1024).is_err());
    }

    #[test]
    fn resource_manager_retries_advertisement_until_budget_exhausted() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let link = Link::new(destination, tx);

        let clock = Arc::new(ManualMonotonicClock::default());
        let mut manager = ResourceManager::new_with_config_and_clock(
            Duration::from_secs(1),
            2,
            clock.clone(),
        );
        let (resource_hash, _) =
            manager.start_send(&link, b"retry me".to_vec(), None).expect("start sender");
        manager.confirm_outbound_dispatch(resource_hash, true);

        clock.advance(Duration::from_secs(2));
        let first = manager.poll_outgoing();
        assert_eq!(first.len(), 1);
        assert!(manager.outgoing.contains_key(&resource_hash));

        clock.advance(Duration::from_secs(2));
        let second = manager.poll_outgoing();
        assert_eq!(second.len(), 1);
        assert!(manager.outgoing.contains_key(&resource_hash));

        clock.advance(Duration::from_secs(2));
        let third = manager.poll_outgoing();
        assert!(third.is_empty());
        assert!(!manager.outgoing.contains_key(&resource_hash));
        assert!(matches!(
            manager.drain_events().as_slice(),
            [ResourceEvent { kind: ResourceEventKind::Failed(ResourceFailure::TimedOut), .. }]
        ));
    }

    #[test]
    fn resource_manager_times_out_transferring_sender_after_retry_budget() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        link.request();

        let clock = Arc::new(ManualMonotonicClock::default());
        let mut manager = ResourceManager::new_with_config_and_clock(
            Duration::from_secs(1),
            1,
            clock.clone(),
        );
        let payload = vec![0x42; PACKET_MDU + 32];
        let (resource_hash, _) = manager.start_send(&link, payload, None).expect("start sender");
        manager.confirm_outbound_dispatch(resource_hash, true);

        let first_map_hash = manager
            .outgoing
            .get(&resource_hash)
            .expect("outgoing sender")
            .map_hashes[0];
        let request = ResourceRequest {
            hashmap_exhausted: false,
            last_map_hash: None,
            resource_hash,
            requested_hashes: vec![first_map_hash],
        };
        let request_packet =
            resource_packet(PacketContext::ResourceRequest, &request.encode(), *link.id());
        let responses = manager.handle_packet(&request_packet, &mut link);

        assert_eq!(responses.len(), 1);
        assert_eq!(
            manager.outgoing.get(&resource_hash).expect("sender").status,
            ResourceStatus::Transferring
        );

        clock.advance(Duration::from_secs(2));
        let first = manager.poll_outgoing();
        assert!(first.is_empty());
        assert!(manager.outgoing.contains_key(&resource_hash));

        clock.advance(Duration::from_secs(2));
        let second = manager.poll_outgoing();
        assert!(second.is_empty());
        assert!(!manager.outgoing.contains_key(&resource_hash));
        assert!(matches!(
            manager.drain_events().as_slice(),
            [ResourceEvent { kind: ResourceEventKind::Failed(ResourceFailure::TimedOut), .. }]
        ));
    }

    #[test]
    fn resource_manager_requests_cached_proof_and_completes_after_replay() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        link.request();

        let clock = Arc::new(ManualMonotonicClock::default());
        let mut manager = ResourceManager::new_with_config_and_clock(
            Duration::from_secs(1),
            3,
            clock.clone(),
        );
        let (resource_hash, _) =
            manager.start_send(&link, b"proof please".to_vec(), None).expect("start sender");
        manager.confirm_outbound_dispatch(resource_hash, true);

        let first_map_hash = manager
            .outgoing
            .get(&resource_hash)
            .expect("outgoing sender")
            .map_hashes[0];
        let request = ResourceRequest {
            hashmap_exhausted: false,
            last_map_hash: None,
            resource_hash,
            requested_hashes: vec![first_map_hash],
        };
        let request_packet =
            resource_packet(PacketContext::ResourceRequest, &request.encode(), *link.id());
        let responses = manager.handle_packet(&request_packet, &mut link);

        assert_eq!(responses.len(), 1);
        assert_eq!(
            manager.outgoing.get(&resource_hash).expect("sender").status,
            ResourceStatus::AwaitingProof
        );

        clock.advance(Duration::from_secs(2));
        let actions = manager.poll();
        let sender = manager.outgoing.get(&resource_hash).expect("awaiting proof sender");
        let proof = ResourceProof { resource_hash, proof: sender.expected_proof };
        let mut proof_packet =
            resource_packet(PacketContext::ResourceProof, &proof.encode(), *link.id());
        proof_packet.header.packet_type = PacketType::Proof;
        let expected_hash = proof_packet.hash();
        assert_eq!(actions.proof_requests, vec![(*link.id(), expected_hash)]);

        let cache_request = build_resource_cache_request_packet(&link, expected_hash)
            .expect("canonical cache request");
        let mut plain = PacketDataBuffer::new();
        let plain_len = link
            .decrypt(cache_request.data.as_slice(), plain.accuire_buf_max())
            .expect("decrypt cache request")
            .len();
        plain.resize(plain_len);
        assert_eq!(cache_request.context, PacketContext::CacheRequest);
        assert_eq!(plain.as_slice(), expected_hash.as_slice());

        manager.handle_packet(&proof_packet, &mut link);
        assert!(!manager.outgoing.contains_key(&resource_hash));
        assert!(matches!(
            manager.drain_events().as_slice(),
            [ResourceEvent { kind: ResourceEventKind::OutboundComplete, .. }]
        ));
    }

    #[test]
    fn cancellation_releases_both_resource_endpoints() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        link.request();
        let mut sender = ResourceManager::new();
        let mut receiver = ResourceManager::new();
        let (resource_hash, advertisement) =
            sender.start_send(&link, vec![0x5a; PACKET_MDU * 2], None).expect("resource");
        sender.confirm_outbound_dispatch(resource_hash, true);

        let mut plain = PacketDataBuffer::new();
        let plain_len = link
            .decrypt(advertisement.data.as_slice(), plain.accuire_buf_max())
            .expect("decrypt advertisement")
            .len();
        plain.resize(plain_len);
        let mut advertisement = advertisement;
        advertisement.data = plain;
        assert_eq!(receiver.handle_packet(&advertisement, &mut link).len(), 1);

        let cancellation = sender.cancel_local(resource_hash).expect("active resource");
        let packet = build_resource_cancel_packet(&link, cancellation.hash, cancellation.context)
            .expect("cancel packet");
        let mut plain = PacketDataBuffer::new();
        let plain_len = link
            .decrypt(packet.data.as_slice(), plain.accuire_buf_max())
            .expect("decrypt cancellation")
            .len();
        plain.resize(plain_len);
        let mut packet = packet;
        packet.data = plain;
        receiver.handle_packet(&packet, &mut link);

        assert_eq!(sender.state_counts().total(), 0);
        assert_eq!(receiver.state_counts().total(), 0);
        assert!(matches!(
            sender.drain_events().as_slice(),
            [ResourceEvent { kind: ResourceEventKind::Failed(ResourceFailure::Cancelled), .. }]
        ));
        let receiver_events = receiver.drain_events();
        assert!(
            receiver_events.iter().any(|event| matches!(
                event.kind,
                ResourceEventKind::Failed(ResourceFailure::Cancelled)
            )),
            "receiver events: {receiver_events:?}"
        );
    }

    #[test]
    fn link_close_cancels_every_direction_once_even_when_hashes_match() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource-link-close"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        link.request();
        let other_signer = PrivateIdentity::new_from_rand(OsRng);
        let other_identity = *other_signer.as_identity();
        let mut other_link = Link::new(
            DestinationDesc {
                identity: other_identity,
                address_hash: other_identity.address_hash,
                name: DestinationName::new("lxmf", "resource-other-link"),
            },
            tokio::sync::broadcast::channel(1).0,
        );
        other_link.request();
        assert_ne!(link.id(), other_link.id());
        let mut manager = ResourceManager::new();

        let (pending_hash, _) =
            manager.start_send(&link, b"pending".to_vec(), None).expect("pending sender");
        let (outgoing_hash, _) =
            manager.start_send(&link, b"outgoing".to_vec(), None).expect("outgoing sender");
        assert!(manager.confirm_outbound_dispatch(outgoing_hash, true));
        let (preserved_hash, _) = manager
            .start_send(&other_link, b"preserved".to_vec(), None)
            .expect("other-link sender");

        let mut same_hash_advertisement = bounded_advertisement(1, 1, link.resource_sdu());
        same_hash_advertisement.hash = outgoing_hash;
        same_hash_advertisement.original_hash = outgoing_hash;
        manager.incoming.insert(
            outgoing_hash,
            ResourceReceiver::new(
                &same_hash_advertisement,
                *link.id(),
                link.resource_sdu(),
                MAX_UNSOLICITED_RESOURCE_SIZE,
                Duration::ZERO,
            )
            .expect("same-hash receiver"),
        );
        let incoming_hash = Hash::new([0xa5; HASH_SIZE]);
        let mut incoming_advertisement = same_hash_advertisement;
        incoming_advertisement.hash = incoming_hash;
        incoming_advertisement.original_hash = incoming_hash;
        manager.incoming.insert(
            incoming_hash,
            ResourceReceiver::new(
                &incoming_advertisement,
                *link.id(),
                link.resource_sdu(),
                MAX_UNSOLICITED_RESOURCE_SIZE,
                Duration::ZERO,
            )
            .expect("incoming receiver"),
        );

        manager.cancel_link(*link.id());
        let events = manager.drain_events();
        assert_eq!(events.len(), 4);
        assert!(events.iter().all(|event| {
            event.link_id == *link.id()
                && matches!(event.kind, ResourceEventKind::Failed(ResourceFailure::LinkClosed))
        }));
        assert_eq!(events.iter().filter(|event| event.hash == pending_hash).count(), 1);
        assert_eq!(events.iter().filter(|event| event.hash == outgoing_hash).count(), 2);
        assert_eq!(events.iter().filter(|event| event.hash == incoming_hash).count(), 1);
        assert_eq!(manager.state_counts().total(), 1);
        assert!(manager.pending_outgoing.contains_key(&preserved_hash));

        manager.cancel_link(*link.id());
        manager.remove_orphaned(&[*other_link.id()]);
        assert!(manager.drain_events().is_empty());
        let actions = manager.poll();
        assert!(actions.requests.is_empty());
        assert!(actions.packets.is_empty());
        assert!(actions.cancellations.is_empty());
        assert!(actions.proof_requests.is_empty());
        assert_eq!(manager.state_counts().total(), 1);
    }

    #[test]
    fn local_cancellation_wins_watchdog_and_duplicate_remote_cancel_race() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource-cancel-race"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        let clock = Arc::new(ManualMonotonicClock::default());
        let mut manager = ResourceManager::new_with_config_and_clock(
            Duration::from_secs(1),
            0,
            clock.clone(),
        );
        let (resource_hash, _) =
            manager.start_send(&link, b"cancel race".to_vec(), None).expect("sender");
        assert!(manager.confirm_outbound_dispatch(resource_hash, true));

        let cancellation = manager.cancel_local(resource_hash).expect("local cancellation");
        let packet = resource_packet(
            PacketContext::ResourceReceiverCancel,
            resource_hash.as_slice(),
            *link.id(),
        );
        manager.handle_packet(&packet, &mut link);
        manager.handle_packet(&packet, &mut link);
        clock.advance(Duration::from_secs(2));
        let actions = manager.poll();

        assert_eq!(cancellation.hash, resource_hash);
        assert!(actions.requests.is_empty());
        assert!(actions.packets.is_empty());
        assert!(actions.cancellations.is_empty());
        assert!(actions.proof_requests.is_empty());
        assert_eq!(manager.state_counts().total(), 0);
        assert!(matches!(
            manager.drain_events().as_slice(),
            [ResourceEvent {
                hash,
                kind: ResourceEventKind::Failed(ResourceFailure::Cancelled),
                ..
            }] if *hash == resource_hash
        ));
    }

    #[test]
    fn receiver_matches_only_the_canonical_requested_window() {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", "resource-window"),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let link = Link::new(destination, tx);
        let random_hash = [0x91; RANDOM_HASH_SIZE];
        let parts = (0_u8..6).map(|index| vec![index; 4]).collect::<Vec<_>>();
        let hashes = parts.iter().map(|part| map_hash(part, &random_hash)).collect::<Vec<_>>();
        let advertisement = ResourceAdvertisement {
            transfer_size: 24,
            data_size: 20,
            parts: 6,
            hash: Hash::new([0x92; HASH_SIZE]),
            random_hash,
            original_hash: Hash::new([0x92; HASH_SIZE]),
            segment_index: 1,
            total_segments: 1,
            request_id: None,
            flags: 0,
            hashmap: hashes.iter().flatten().copied().collect(),
        };
        let mut receiver =
            ResourceReceiver::new(&advertisement, *link.id(), 4, 1024, Duration::ZERO)
                .expect("bounded receiver");

        assert_eq!(receiver.build_request().requested_hashes, hashes[..4]);
        assert!(matches!(
            receiver.handle_part(&parts[4], &link, Duration::from_secs(1)),
            PartOutcome::NoMatch
        ));
        assert!(matches!(
            receiver.handle_part(&parts[3], &link, Duration::from_secs(2)),
            PartOutcome::Incomplete
        ));
        assert_eq!(receiver.build_request().requested_hashes, hashes[..3]);

        for (offset, part) in parts[..3].iter().enumerate() {
            assert!(matches!(
                receiver.handle_part(part, &link, Duration::from_secs(3 + offset as u64)),
                PartOutcome::Incomplete
            ));
        }
        assert_eq!(receiver.consecutive_completed, 4);
        assert_eq!(receiver.build_request().requested_hashes, hashes[4..]);
    }

    fn resource_packet(context: PacketContext, payload: &[u8], destination: AddressHash) -> Packet {
        Packet {
            header: Header {
                destination_type: DestinationType::Link,
                packet_type: PacketType::Data,
                ..Default::default()
            },
            destination,
            context,
            data: PacketDataBuffer::new_from_slice(payload),
            ..Default::default()
        }
    }

    /// A plain, uncompressed resource of `part_count` fragments of `sdu`
    /// bytes whose advertisement hash matches the assembled payload, so a
    /// receiver can run to verified completion.
    fn windowed_resource(
        part_count: usize,
        sdu: usize,
        seed: u8,
    ) -> (Vec<Vec<u8>>, Vec<[u8; MAPHASH_LEN]>, ResourceAdvertisement) {
        let random_hash = [seed; RANDOM_HASH_SIZE];
        let parts = (0..part_count)
            .map(|index| vec![index as u8 ^ seed; sdu])
            .collect::<Vec<_>>();
        let hashes = parts.iter().map(|part| map_hash(part, &random_hash)).collect::<Vec<_>>();
        let stream = parts.concat();
        let payload = &stream[RANDOM_HASH_SIZE..];
        let mut hasher = sha2::Sha256::new();
        hasher.update(payload);
        hasher.update(random_hash);
        let hash = Hash::new(copy_hash(&hasher.finalize()).expect("hash size"));
        let advertisement = ResourceAdvertisement {
            transfer_size: stream.len() as u64,
            data_size: payload.len() as u64,
            parts: part_count as u32,
            hash,
            random_hash,
            original_hash: hash,
            segment_index: 1,
            total_segments: part_count.div_ceil(HASHMAP_MAX_LEN) as u32,
            request_id: None,
            flags: 0,
            hashmap: hashes[..part_count.min(HASHMAP_MAX_LEN)].iter().flatten().copied().collect(),
        };
        (parts, hashes, advertisement)
    }

    fn hashmap_continuation(
        resource_hash: Hash,
        hashes: &[[u8; MAPHASH_LEN]],
        segment: usize,
    ) -> ResourceHashUpdate {
        let start = segment * HASHMAP_MAX_LEN;
        let end = (start + HASHMAP_MAX_LEN).min(hashes.len());
        ResourceHashUpdate {
            resource_hash,
            segment: segment as u32,
            hashmap: hashes[start..end].iter().flatten().copied().collect(),
        }
    }

    /// Deliver `parts[..count]` in order without any request accounting so the
    /// receiver's consecutive height advances to `count`.
    fn advance_receiver(receiver: &mut ResourceReceiver, link: &Link, parts: &[Vec<u8>], count: usize) {
        for (offset, part) in parts[..count].iter().enumerate() {
            assert!(matches!(
                receiver.handle_part(part, link, Duration::from_millis(offset as u64)),
                PartOutcome::Incomplete
            ));
        }
        assert_eq!(receiver.consecutive_completed, count);
    }

    fn requested_link(name: &str) -> Link {
        let signer = PrivateIdentity::new_from_rand(OsRng);
        let identity = *signer.as_identity();
        let destination = DestinationDesc {
            identity,
            address_hash: identity.address_hash,
            name: DestinationName::new("lxmf", name),
        };
        let (tx, _) = tokio::sync::broadcast::channel(1);
        let mut link = Link::new(destination, tx);
        link.request();
        link
    }

    #[test]
    fn fragment_progress_never_consumes_retries_and_requests_once_per_drained_round() {
        let link = requested_link("rounds");
        let (parts, hashes, advertisement) = windowed_resource(12, 4, 0x41);
        let mut receiver =
            ResourceReceiver::new(&advertisement, *link.id(), 4, 1024, Duration::ZERO)
                .expect("bounded receiver");

        let initial = receiver
            .request_round(RequestRound::Initial, Duration::ZERO)
            .expect("initial request");
        assert_eq!(initial.requested_hashes, hashes[..4]);
        assert_eq!(receiver.retry_count, 0);
        assert!(!receiver.round_drained());

        for (offset, part) in parts[..3].iter().enumerate() {
            let now = Duration::from_millis(100 * (offset as u64 + 1));
            assert!(matches!(receiver.handle_part(part, &link, now), PartOutcome::Incomplete));
            assert!(!receiver.round_drained(), "round drains only once every fragment arrived");
            assert_eq!(receiver.retry_count, 0, "arriving fragments never consume retries");
        }
        assert!(matches!(
            receiver.handle_part(&parts[3], &link, Duration::from_millis(400)),
            PartOutcome::Incomplete
        ));
        assert!(receiver.round_drained());
        assert!(matches!(
            receiver.handle_part(&parts[3], &link, Duration::from_millis(450)),
            PartOutcome::NoMatch
        ));
        assert!(receiver.round_drained(), "a duplicate fragment does not reopen the round");

        let next = receiver
            .request_round(RequestRound::Drained, Duration::from_millis(500))
            .expect("drained round request");
        assert_eq!(receiver.window, WINDOW + 1, "a clean round grows the window by one");
        assert_eq!(next.requested_hashes, hashes[4..9]);
        assert_eq!(receiver.retry_count, 0);
    }

    #[test]
    fn clean_rounds_grow_the_window_to_its_ceiling_and_complete_the_transfer() {
        let link = requested_link("ceiling");
        let (parts, hashes, advertisement) = windowed_resource(64, 4, 0x17);
        let mut receiver =
            ResourceReceiver::new(&advertisement, *link.id(), 4, 1024, Duration::ZERO)
                .expect("bounded receiver");
        let mut now = Duration::ZERO;
        let mut request =
            receiver.request_round(RequestRound::Initial, now).expect("initial request");
        let mut windows = vec![receiver.window];
        let mut next_index = 0usize;
        loop {
            assert_eq!(request.requested_hashes.len(), receiver.window.min(64 - next_index));
            assert_eq!(
                request.requested_hashes,
                hashes[next_index..next_index + request.requested_hashes.len()]
            );
            let mut completed = false;
            for _ in 0..request.requested_hashes.len() {
                now += Duration::from_millis(10);
                match receiver.handle_part(&parts[next_index], &link, now) {
                    PartOutcome::Incomplete => {}
                    PartOutcome::Complete(proof, payload) => {
                        assert_eq!(proof.context, PacketContext::ResourceProof);
                        assert_eq!(payload.data, parts.concat()[RANDOM_HASH_SIZE..].to_vec());
                        completed = true;
                    }
                    other => panic!("unexpected outcome {:?}", std::mem::discriminant(&other)),
                }
                next_index += 1;
            }
            if completed {
                break;
            }
            assert!(receiver.round_drained());
            request =
                receiver.request_round(RequestRound::Drained, now).expect("drained round request");
            windows.push(receiver.window);
        }
        assert_eq!(next_index, 64);
        assert_eq!(receiver.retry_count, 0);
        assert_eq!(windows, vec![4, 5, 6, 7, 8, 9, 10, 10, 10]);
        assert_eq!(receiver.status, ResourceStatus::Complete);
    }

    #[test]
    fn timed_out_rounds_shrink_the_window_to_its_floor_and_exhaust_retries() {
        let mut link = requested_link("timeouts");
        let sdu = link.resource_sdu();
        let (_, hashes, advertisement) = windowed_resource(16, sdu, 0x23);
        let clock = Arc::new(ManualMonotonicClock::default());
        let mut manager = ResourceManager::new_with_config_and_clock(
            Duration::from_secs(1),
            4,
            clock.clone(),
        );
        let packet = resource_packet(
            PacketContext::ResourceAdvrtisement,
            &advertisement.pack().expect("advertisement"),
            *link.id(),
        );
        let responses = manager.handle_packet(&packet, &mut link);
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].context, PacketContext::ResourceRequest);
        assert_eq!(manager.incoming[&advertisement.hash].outstanding.len(), 4);

        let mut expected_windows = Vec::new();
        for (retry, expected_window) in [(1_u8, 3_usize), (2, 2), (3, 1), (4, 1)] {
            clock.advance(Duration::from_millis(999));
            assert!(manager.poll().requests.is_empty(), "no retry before the interval");
            clock.advance(Duration::from_millis(1));
            let actions = manager.poll();
            assert_eq!(actions.requests.len(), 1);
            assert_eq!(actions.requests[0].request.requested_hashes, hashes[..expected_window]);
            let receiver = manager.incoming.get(&advertisement.hash).expect("receiver");
            assert_eq!(receiver.retry_count, retry);
            expected_windows.push(receiver.window);
        }
        assert_eq!(expected_windows, vec![3, 2, 1, 1], "the window never shrinks below its floor");

        clock.advance(Duration::from_secs(1));
        let actions = manager.poll();
        assert!(actions.requests.is_empty());
        assert_eq!(actions.cancellations.len(), 1);
        assert!(!manager.incoming.contains_key(&advertisement.hash));
        assert!(matches!(
            manager.drain_events().as_slice(),
            [ResourceEvent { kind: ResourceEventKind::Failed(ResourceFailure::TimedOut), .. }]
        ));
    }

    #[test]
    fn progress_resets_the_retry_budget_and_the_manager_requests_once_per_round() {
        let mut link = requested_link("budget");
        let sdu = link.resource_sdu();
        let (parts, hashes, advertisement) = windowed_resource(16, sdu, 0x59);
        let clock = Arc::new(ManualMonotonicClock::default());
        let mut manager = ResourceManager::new_with_config_and_clock(
            Duration::from_secs(1),
            2,
            clock.clone(),
        );
        let packet = resource_packet(
            PacketContext::ResourceAdvrtisement,
            &advertisement.pack().expect("advertisement"),
            *link.id(),
        );
        assert_eq!(manager.handle_packet(&packet, &mut link).len(), 1);

        clock.advance(Duration::from_secs(1));
        let retry = manager.poll();
        assert_eq!(retry.requests.len(), 1);
        assert_eq!(retry.requests[0].request.requested_hashes, hashes[..3]);
        assert_eq!(manager.incoming[&advertisement.hash].retry_count, 1);

        clock.advance(Duration::from_millis(100));
        for part in &parts[..2] {
            let part = resource_packet(PacketContext::Resource, part, *link.id());
            assert!(
                manager.handle_packet(&part, &mut link).is_empty(),
                "fragments still in flight are not requested again"
            );
            assert_eq!(manager.incoming[&advertisement.hash].retry_count, 0);
        }
        let events = manager.drain_events();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| matches!(event.kind, ResourceEventKind::Progress(_))));

        let part = resource_packet(PacketContext::Resource, &parts[2], *link.id());
        let responses = manager.handle_packet(&part, &mut link);
        assert_eq!(responses.len(), 1, "one request per drained round");
        assert_eq!(responses[0].context, PacketContext::ResourceRequest);
        assert_eq!(manager.incoming[&advertisement.hash].window, 4);
        assert_eq!(manager.incoming[&advertisement.hash].outstanding.len(), 4);

        clock.advance(Duration::from_secs(1));
        let poll = manager.poll();
        assert_eq!(poll.requests.len(), 1, "a stalled round retries again");
        assert_eq!(manager.incoming[&advertisement.hash].retry_count, 1);
    }

    #[test]
    fn continuation_refill_never_requests_an_in_flight_fragment_twice() {
        let boundary = HASHMAP_MAX_LEN;
        let link = requested_link("refill");
        let (parts, hashes, advertisement) = windowed_resource(boundary + 6, 4, 0x61);
        let mut receiver =
            ResourceReceiver::new(&advertisement, *link.id(), 4, 4096, Duration::ZERO)
                .expect("multi-segment receiver");
        advance_receiver(&mut receiver, &link, &parts, boundary - 2);

        let request = receiver
            .request_round(RequestRound::Drained, Duration::from_secs(1))
            .expect("window reaches the segment boundary");
        assert_eq!(receiver.window, WINDOW + 1);
        assert_eq!(request.requested_hashes, hashes[boundary - 2..boundary]);
        assert!(request.hashmap_exhausted, "the active window reached unmapped fragments");
        assert_eq!(request.last_map_hash, Some(hashes[boundary - 1]));
        assert!(receiver.continuation_pending);

        assert!(matches!(
            receiver.handle_part(&parts[boundary - 2], &link, Duration::from_millis(1100)),
            PartOutcome::Incomplete
        ));
        assert!(!receiver.round_drained(), "one requested fragment is still in flight");

        receiver.handle_hash_update(&hashmap_continuation(advertisement.hash, &hashes, 1));
        assert!(!receiver.continuation_pending);
        let refill = receiver
            .request_round(RequestRound::Continuation, Duration::from_millis(1200))
            .expect("continuation refills the window");
        assert!(!refill.requested_hashes.contains(&hashes[boundary - 1]), "in flight, not re-requested");
        assert_eq!(refill.requested_hashes, hashes[boundary..boundary + 4], "refill stays within the window");
        assert!(!refill.hashmap_exhausted);
        assert_eq!(receiver.outstanding.len(), receiver.window);
        assert_eq!(receiver.retry_count, 0);
    }

    #[test]
    fn one_continuation_stays_outstanding_until_it_arrives_or_expires() {
        let boundary = HASHMAP_MAX_LEN;
        let link = requested_link("continuation");
        let (parts, hashes, advertisement) = windowed_resource(boundary + 2, 4, 0x77);
        let mut receiver =
            ResourceReceiver::new(&advertisement, *link.id(), 4, 4096, Duration::ZERO)
                .expect("multi-segment receiver");
        advance_receiver(&mut receiver, &link, &parts, boundary - 1);

        let request = receiver
            .request_round(RequestRound::Drained, Duration::from_secs(1))
            .expect("first continuation request");
        assert_eq!(request.requested_hashes, hashes[boundary - 1..boundary]);
        assert!(request.hashmap_exhausted);

        assert!(matches!(
            receiver.handle_part(&parts[boundary - 1], &link, Duration::from_millis(1100)),
            PartOutcome::Incomplete
        ));
        assert!(receiver.round_drained());
        assert!(
            receiver.request_round(RequestRound::Drained, Duration::from_millis(1200)).is_none(),
            "no second continuation while one is outstanding"
        );
        assert_eq!(receiver.last_request, Duration::from_secs(1), "nothing was sent");
        assert!(receiver.continuation_pending);

        let retry = receiver
            .request_round(RequestRound::Retry, Duration::from_millis(2200))
            .expect("an expired continuation is requested again");
        assert!(retry.hashmap_exhausted);
        assert_eq!(retry.last_map_hash, Some(hashes[boundary - 1]));
        assert!(retry.requested_hashes.is_empty());
        assert_eq!(receiver.retry_count, 1);
        assert!(receiver.continuation_pending);

        receiver.handle_hash_update(&hashmap_continuation(advertisement.hash, &hashes, 1));
        let refill = receiver
            .request_round(RequestRound::Continuation, Duration::from_millis(2300))
            .expect("continuation arrived");
        assert_eq!(refill.requested_hashes, hashes[boundary..]);
        assert!(!refill.hashmap_exhausted);
    }

    #[test]
    fn lost_continuation_expires_into_bounded_re_requests_and_one_terminal_timeout() {
        let boundary = HASHMAP_MAX_LEN;
        let mut link = requested_link("lost-continuation");
        let sdu = link.resource_sdu();
        let (parts, hashes, advertisement) = windowed_resource(boundary + 2, sdu, 0x13);
        let clock = Arc::new(ManualMonotonicClock::default());
        let mut manager = ResourceManager::new_with_config_and_clock(
            Duration::from_secs(1),
            2,
            clock.clone(),
        );
        let packet = resource_packet(
            PacketContext::ResourceAdvrtisement,
            &advertisement.pack().expect("advertisement"),
            *link.id(),
        );
        assert_eq!(manager.handle_packet(&packet, &mut link).len(), 1);

        let mut requests = 0usize;
        for part in &parts[..boundary] {
            clock.advance(Duration::from_millis(1));
            let packet = resource_packet(PacketContext::Resource, part, *link.id());
            let responses = manager.handle_packet(&packet, &mut link);
            assert!(responses.len() <= 1);
            requests += responses.len();
            let receiver = &manager.incoming[&advertisement.hash];
            assert!(receiver.outstanding.len() <= receiver.window, "requests stay bounded");
        }
        assert!(requests > 0);
        let receiver = &manager.incoming[&advertisement.hash];
        assert_eq!(receiver.consecutive_completed, boundary);
        assert!(receiver.continuation_pending, "the window reached the next segment");
        assert!(receiver.round_drained());
        let _ = manager.drain_events();

        for retry in 1..=2_u8 {
            clock.advance(Duration::from_secs(1));
            let actions = manager.poll();
            assert_eq!(actions.requests.len(), 1, "the expired continuation is requested again");
            assert!(actions.requests[0].request.hashmap_exhausted);
            assert_eq!(actions.requests[0].request.last_map_hash, Some(hashes[boundary - 1]));
            assert_eq!(manager.incoming[&advertisement.hash].retry_count, retry);
        }
        clock.advance(Duration::from_secs(1));
        let actions = manager.poll();
        assert!(actions.requests.is_empty());
        assert_eq!(actions.cancellations.len(), 1);
        assert!(!manager.incoming.contains_key(&advertisement.hash), "owned state released");
        assert!(matches!(
            manager.drain_events().as_slice(),
            [ResourceEvent { kind: ResourceEventKind::Failed(ResourceFailure::TimedOut), .. }]
        ));
        assert!(manager.poll().cancellations.is_empty(), "exactly one terminal failure");
    }

    #[test]
    fn multi_segment_transfer_completes_and_releases_request_state() {
        let boundary = HASHMAP_MAX_LEN;
        let mut link = requested_link("multi-segment");
        let sdu = link.resource_sdu();
        let (parts, hashes, advertisement) = windowed_resource(boundary + 3, sdu, 0x2b);
        let clock = Arc::new(ManualMonotonicClock::default());
        let mut manager = ResourceManager::new_with_config_and_clock(
            Duration::from_secs(1),
            2,
            clock.clone(),
        );
        let packet = resource_packet(
            PacketContext::ResourceAdvrtisement,
            &advertisement.pack().expect("advertisement"),
            *link.id(),
        );
        assert_eq!(manager.handle_packet(&packet, &mut link).len(), 1);
        for part in &parts[..boundary] {
            let packet = resource_packet(PacketContext::Resource, part, *link.id());
            manager.handle_packet(&packet, &mut link);
        }
        assert!(manager.incoming[&advertisement.hash].continuation_pending);

        let update = hashmap_continuation(advertisement.hash, &hashes, 1);
        let packet = resource_packet(
            PacketContext::ResourceHashUpdate,
            &update.encode().expect("hash update encodes"),
            *link.id(),
        );
        let responses = manager.handle_packet(&packet, &mut link);
        assert_eq!(responses.len(), 1, "the continuation refills the window once");
        assert_eq!(responses[0].context, PacketContext::ResourceRequest);
        {
            let receiver = &manager.incoming[&advertisement.hash];
            assert!(!receiver.continuation_pending);
            assert_eq!(receiver.outstanding.len(), 3);
            assert_eq!(receiver.retry_count, 0);
        }

        let mut responses = Vec::new();
        for part in &parts[boundary..] {
            let packet = resource_packet(PacketContext::Resource, part, *link.id());
            responses = manager.handle_packet(&packet, &mut link);
        }
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].context, PacketContext::ResourceProof);
        assert!(manager.incoming.is_empty(), "completion releases owned request state");
        assert!(manager
            .drain_events()
            .iter()
            .any(|event| matches!(event.kind, ResourceEventKind::Complete(_))));
        clock.advance(Duration::from_secs(5));
        assert!(manager.poll().cancellations.is_empty());
    }
}
