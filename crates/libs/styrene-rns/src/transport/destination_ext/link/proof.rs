#[allow(dead_code)] // Used in trace logging paths
fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:02x}", byte);
    }
    out
}

pub(crate) fn validate_link_request_proof_packet(
    destination: &DestinationDesc,
    id: &LinkId,
    packet: &Packet,
) -> Result<(Identity, Option<LinkSignalling>), RnsError> {
    const MIN_PROOF_LEN: usize = SIGNATURE_LENGTH + PUBLIC_KEY_LENGTH;
    const MTU_PROOF_LEN: usize = SIGNATURE_LENGTH + PUBLIC_KEY_LENGTH + LINK_MTU_SIZE;
    const SIGN_DATA_LEN: usize = ADDRESS_HASH_SIZE + PUBLIC_KEY_LENGTH * 2 + LINK_MTU_SIZE;

    if !matches!(packet.data.len(), MIN_PROOF_LEN | MTU_PROOF_LEN) {
        return Err(RnsError::PacketError);
    }

    let mut proof_data = [0u8; SIGN_DATA_LEN];

    let verifying_key = destination.identity.verifying_key.as_bytes();
    let sign_data_len = {
        let mut output = OutputBuffer::new(&mut proof_data[..]);

        output.write(id.as_slice())?;
        output.write(
            &packet.data.as_slice()[SIGNATURE_LENGTH..SIGNATURE_LENGTH + PUBLIC_KEY_LENGTH],
        )?;
        output.write(verifying_key)?;

        if packet.data.len() == MTU_PROOF_LEN {
            output.write(
                &packet.data.as_slice()[SIGNATURE_LENGTH + PUBLIC_KEY_LENGTH..MTU_PROOF_LEN],
            )?;
        }

        output.offset()
    };

    let identity = Identity::new_from_slices(
        &proof_data[ADDRESS_HASH_SIZE..ADDRESS_HASH_SIZE + PUBLIC_KEY_LENGTH],
        verifying_key,
    );

    let signature = Signature::from_slice(&packet.data.as_slice()[..SIGNATURE_LENGTH])
        .map_err(|_| RnsError::CryptoError)?;

    identity
        .verify(&proof_data[..sign_data_len], &signature)
        .map_err(|_| RnsError::IncorrectSignature)?;

    let signalling = if packet.data.len() == MTU_PROOF_LEN {
        let mut bytes = [0_u8; LINK_MTU_SIZE];
        bytes.copy_from_slice(&packet.data.as_slice()[MIN_PROOF_LEN..MTU_PROOF_LEN]);
        Some(LinkSignalling::decode(bytes)?)
    } else {
        None
    };

    Ok((identity, signalling))
}

#[cfg(test)]
mod canonical_context_tests {
    use super::*;
    use crate::identity::PrivateIdentity;
    use crate::packet::{Header, PacketDataBuffer};
    use rand_core::OsRng;

    fn link_proof(id: &LinkId, context: PacketContext, data: &[u8]) -> Packet {
        Packet {
            header: Header {
                packet_type: PacketType::Proof,
                destination_type: DestinationType::Link,
                ..Default::default()
            },
            destination: *id,
            context,
            data: PacketDataBuffer::new_from_slice(data),
            ..Default::default()
        }
    }

    #[test]
    fn link_packet_proofs_accept_canonical_and_local_contexts() {
        let peer = PrivateIdentity::new_from_rand(OsRng);
        let id = LinkId::new_from_rand(OsRng);
        let packet_hash = [0x3cu8; HASH_SIZE];
        let mut data = Vec::from(packet_hash);
        data.extend_from_slice(&peer.sign(&packet_hash).to_bytes());

        for context in [PacketContext::None, PacketContext::LinkProof] {
            let proof = link_proof(&id, context, &data);
            let validated = validate_link_packet_proof(peer.as_identity(), &id, &proof)
                .unwrap_or_else(|error| panic!("{context:?} rejected: {error:?}"));
            assert_eq!(validated.to_bytes(), packet_hash);
        }

        let wrong_context = link_proof(&id, PacketContext::LinkRequestProof, &data);
        assert!(validate_link_packet_proof(peer.as_identity(), &id, &wrong_context).is_err());
        let other_link = LinkId::new_from_rand(OsRng);
        let wrong_link = link_proof(&other_link, PacketContext::None, &data);
        assert!(validate_link_packet_proof(peer.as_identity(), &id, &wrong_link).is_err());
        let stranger = PrivateIdentity::new_from_rand(OsRng);
        let mut forged = Vec::from(packet_hash);
        forged.extend_from_slice(&stranger.sign(&packet_hash).to_bytes());
        let forged = link_proof(&id, PacketContext::None, &forged);
        assert!(validate_link_packet_proof(peer.as_identity(), &id, &forged).is_err());
    }
}

pub(crate) fn validate_link_packet_proof(
    identity: &Identity,
    id: &LinkId,
    packet: &Packet,
) -> Result<Hash, RnsError> {
    if packet.header.packet_type != PacketType::Proof
        || packet.header.destination_type != DestinationType::Link
        || !matches!(packet.context, PacketContext::LinkProof | PacketContext::None)
    {
        return Err(RnsError::PacketError);
    }
    if packet.destination != *id {
        return Err(RnsError::IncorrectHash);
    }
    if packet.data.len() < HASH_SIZE + SIGNATURE_LENGTH {
        return Err(RnsError::PacketError);
    }

    let mut hash = [0u8; HASH_SIZE];
    hash.copy_from_slice(&packet.data.as_slice()[..HASH_SIZE]);
    let signature =
        Signature::from_slice(&packet.data.as_slice()[HASH_SIZE..HASH_SIZE + SIGNATURE_LENGTH])
            .map_err(|_| RnsError::CryptoError)?;

    identity.verify(&hash, &signature)?;

    Ok(Hash::new(hash))
}
