use std::io::Cursor;

use rmpv::Value;

use super::*;
use crate::destination::RequestLinkContext;
use crate::transport::destination_ext::link::LinkPayload;
use crate::transport::request::{decode_response_envelope, encode_response_envelope};
use crate::transport::resource::{ResourceAdvertisement, ResourceEvent, ResourceEventKind};

struct DecodedRequest<'a> {
    path_hash: RequestPathHash,
    data: &'a [u8],
}

fn decode_request(payload: &[u8]) -> Option<DecodedRequest<'_>> {
    let mut cursor = Cursor::new(payload);
    let array_len = rmp::decode::read_array_len(&mut cursor).ok()?;
    if array_len != 3 {
        return None;
    }

    let requested_at = rmpv::decode::read_value(&mut cursor).ok()?;
    if !matches!(requested_at, Value::F32(_) | Value::F64(_) | Value::Integer(_)) {
        return None;
    }
    let path = rmpv::decode::read_value(&mut cursor).ok()?;
    let Value::Binary(path) = path else { return None };
    let path_hash: RequestPathHash = path.try_into().ok()?;

    // Preserve the third MessagePack object exactly so handlers can decode
    // application-specific request data without a lossy intermediate model.
    let data_start = usize::try_from(cursor.position()).ok()?;
    rmpv::decode::read_value(&mut cursor).ok()?;
    let data_end = usize::try_from(cursor.position()).ok()?;
    if data_end != payload.len() {
        return None;
    }

    Some(DecodedRequest { path_hash, data: &payload[data_start..data_end] })
}

pub(super) fn correlate_packet_response(
    handler: &mut TransportHandler,
    link_id: LinkId,
    payload: &[u8],
) {
    match decode_response_envelope(payload) {
        Ok((request_id, response)) => {
            handler.request_tracker.packet_response(
                link_id,
                request_id,
                response,
                payload.len() as u64,
            );
        }
        Err(Some(request_id)) => {
            handler.request_tracker.malformed(link_id, request_id);
        }
        Err(None) => {}
    }
}

impl TransportHandler {
    pub(super) fn correlate_resource_advertisement(
        &mut self,
        link_id: LinkId,
        payload: &[u8],
    ) -> Option<Hash> {
        let Ok(advertisement) = ResourceAdvertisement::unpack(payload) else {
            return None;
        };
        if !advertisement.is_response() {
            return None;
        }
        let Some(request_id) =
            advertisement.request_id.as_ref().and_then(|value| value.as_slice().try_into().ok())
        else {
            return Some(advertisement.hash);
        };
        let Some(receipt) = self.request_tracker.get(&request_id) else {
            return Some(advertisement.hash);
        };
        if receipt.link_id != link_id || receipt.status.is_terminal() {
            return Some(advertisement.hash);
        }
        let maximum_resource_size = receipt
            .maximum_response_size()
            .checked_add(RESPONSE_ENVELOPE_OVERHEAD as usize)
            .filter(|limit| *limit <= crate::transport::resource::MAX_NEGOTIATED_RESOURCE_SIZE);
        let Some(maximum_resource_size) = maximum_resource_size else {
            return Some(advertisement.hash);
        };
        const RESPONSE_ENVELOPE_OVERHEAD: u64 = 19;
        if let Ok(size) =
            usize::try_from(advertisement.data_size.saturating_sub(RESPONSE_ENVELOPE_OVERHEAD))
        {
            if !self.request_tracker.resource_advertised(
                link_id,
                request_id,
                advertisement.hash,
                size,
                advertisement.transfer_size,
            ) {
                return Some(advertisement.hash);
            }
            if !self.resource_manager.set_incoming_limit(advertisement.hash, maximum_resource_size)
            {
                return Some(advertisement.hash);
            }
        }
        None
    }

    pub(super) async fn publish_resource_events(&mut self, events: Vec<ResourceEvent>) {
        for mut event in events {
            match &event.kind {
                ResourceEventKind::Progress(progress) => {
                    self.request_tracker.resource_progress(
                        event.link_id,
                        event.hash,
                        progress.received_bytes,
                        progress.total_bytes,
                    );
                }
                ResourceEventKind::Complete(complete) => {
                    if complete.is_response {
                        let expected_id =
                            self.request_tracker.response_resource_request_id(event.hash);
                        match decode_response_envelope(&complete.data) {
                            Ok((request_id, response))
                                if expected_id == Some(request_id)
                                    && complete.request_id == Some(request_id) =>
                            {
                                self.request_tracker.resource_complete(
                                    event.link_id,
                                    event.hash,
                                    response.clone(),
                                    complete.transfer_size,
                                );
                                if let ResourceEventKind::Complete(complete) = &mut event.kind {
                                    complete.data = response;
                                }
                            }
                            Err(_) | Ok(_) => {
                                if let Some(request_id) = expected_id {
                                    self.request_tracker.malformed(event.link_id, request_id);
                                }
                            }
                        }
                        continue;
                    }
                    if complete.is_request {
                        self.handle_completed_request_resource(event.link_id, complete).await;
                        continue;
                    }
                }
                ResourceEventKind::Failed(_) => {
                    self.request_tracker.resource_failed(event.link_id, event.hash);
                }
                ResourceEventKind::OutboundComplete => {}
            }
            let _ = self.resource_events_tx.send(event);
        }
    }

    async fn handle_completed_request_resource(
        &mut self,
        link_id: LinkId,
        complete: &crate::transport::resource::ResourceComplete,
    ) {
        let Some(request_id) = complete.request_id else { return };
        if crate::hash::address_hash(&complete.data) != request_id {
            return;
        }
        let Some(link) = super::links::find_link_in_handler(self, link_id).await else { return };
        let (destination_hash, remote_identity, iface) = {
            let link = link.lock().await;
            (link.destination().address_hash, link.remote_identity().copied(), link.ingress_iface())
        };
        let Some(iface) = iface else { return };
        let destination = self.single_in_destinations.get(&destination_hash).cloned();
        let event = dispatch_request_data(
            destination,
            destination_hash,
            link_id,
            remote_identity,
            request_id,
            &complete.data,
        )
        .await;
        send_server_response(self, &event, iface).await;
        let _ = self.server_request_tx.send(event);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerResponseMode {
    Packet,
    Resource,
}

fn server_response_mode(encoded_size: usize, packet_mdu: usize) -> ServerResponseMode {
    if encoded_size <= packet_mdu {
        ServerResponseMode::Packet
    } else {
        ServerResponseMode::Resource
    }
}

pub(super) async fn send_server_response(
    handler: &mut TransportHandler,
    event: &ServerRequestEvent,
    iface: AddressHash,
) {
    let Some(request_id) = event.request_id else { return };
    let Some(link) = super::links::find_link_in_handler(handler, event.link_id).await else {
        return;
    };
    let ServerRequestOutcome::Handled(response) = &event.outcome else { return };
    let Some(envelope) = encode_response_envelope(request_id, response) else { return };

    let link = link.lock().await;
    if server_response_mode(envelope.len(), link.packet_mdu()) == ServerResponseMode::Packet {
        let packet = { link.response_packet(&envelope).ok() };
        drop(link);
        if let Some(packet) = packet {
            handler.send(TxMessage { tx_type: TxMessageType::Direct(iface), packet }).await;
        }
    } else {
        if let Ok((hash, packet)) =
            handler.resource_manager.start_response(&link, envelope, request_id)
        {
            drop(link);
            let sent = handler
                .send(TxMessage { tx_type: TxMessageType::Direct(iface), packet })
                .await
                .sent_ifaces
                > 0;
            handler.resource_manager.confirm_outbound_dispatch(hash, sent);
        }
    }
}

pub(super) async fn dispatch_link_request(
    destination: Option<Arc<Mutex<SingleInputDestination>>>,
    destination_hash: AddressHash,
    link_id: LinkId,
    remote_identity: Option<Identity>,
    payload: &LinkPayload,
) -> ServerRequestEvent {
    let malformed = || ServerRequestEvent {
        destination: destination_hash,
        link_id,
        request_id: payload.request_id(),
        path_hash: None,
        outcome: ServerRequestOutcome::Malformed,
    };
    let Some(request_id) = payload.request_id() else { return malformed() };
    dispatch_request_data(
        destination,
        destination_hash,
        link_id,
        remote_identity,
        request_id,
        payload.as_slice(),
    )
    .await
}

async fn dispatch_request_data(
    destination: Option<Arc<Mutex<SingleInputDestination>>>,
    destination_hash: AddressHash,
    link_id: LinkId,
    remote_identity: Option<Identity>,
    request_id: RequestId,
    payload: &[u8],
) -> ServerRequestEvent {
    let malformed = || ServerRequestEvent {
        destination: destination_hash,
        link_id,
        request_id: Some(request_id),
        path_hash: None,
        outcome: ServerRequestOutcome::Malformed,
    };
    let Some(decoded) = decode_request(payload) else { return malformed() };

    let Some(destination) = destination else {
        return ServerRequestEvent {
            destination: destination_hash,
            link_id,
            request_id: Some(request_id),
            path_hash: Some(decoded.path_hash),
            outcome: ServerRequestOutcome::PathNotFound,
        };
    };
    let link_context = RequestLinkContext { link_id, destination: destination_hash };
    let outcome = destination
        .lock()
        .await
        .dispatch_request_with_size(
            &decoded.path_hash,
            decoded.data,
            payload.len(),
            remote_identity.as_ref(),
            &link_context,
            request_id,
        )
        .map(ServerRequestOutcome::Handled)
        .unwrap_or_else(ServerRequestOutcome::from);

    ServerRequestEvent {
        destination: destination_hash,
        link_id,
        request_id: Some(request_id),
        path_hash: Some(decoded.path_hash),
        outcome,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use rand_core::OsRng;
    use tokio::time::{Duration, timeout};

    use super::*;
    use crate::destination::{
        DestinationName, RequestAccess, RequestHandler, RequestRegistrationError,
        SingleInputDestination, request_path_hash,
    };
    use crate::identity::PrivateIdentity;
    use crate::packet::PacketContext;
    use crate::transport::destination_ext::link::{Link, LinkHandleResult};

    struct LinkedServer {
        transport: Transport,
        destination: Arc<Mutex<SingleInputDestination>>,
        outbound: Link,
        inbound: Arc<Mutex<Link>>,
        iface: AddressHash,
    }

    async fn linked_server() -> LinkedServer {
        let local = PrivateIdentity::new_from_rand(OsRng);
        let mut transport = Transport::new(TransportConfig::new("request-server", &local, true));
        let destination = transport
            .add_destination(
                PrivateIdentity::new_from_rand(OsRng),
                DestinationName::new("nomadnetwork", "node"),
            )
            .await;
        let desc = destination.lock().await.desc;
        let signing_key = destination.lock().await.sign_key().clone();
        let mut outbound = Link::new(desc, transport.link_out_event_tx.clone());
        let link_request = outbound.request();
        let mut inbound = Link::new_from_request(
            &link_request,
            signing_key,
            desc,
            transport.link_in_event_tx.clone(),
        )
        .expect("canonical link request");
        let iface = AddressHash::new_from_rand(OsRng);
        assert!(matches!(
            outbound.handle_packet(&inbound.prove(), iface),
            LinkHandleResult::Activated
        ));
        inbound.set_ingress_iface(iface);
        let inbound = Arc::new(Mutex::new(inbound));
        transport.handler.lock().await.in_links.insert(*outbound.id(), inbound.clone());

        LinkedServer { transport, destination, outbound, inbound, iface }
    }

    fn request_envelope(path: &str, data: &[u8]) -> Vec<u8> {
        let mut envelope = Vec::new();
        rmp::encode::write_array_len(&mut envelope, 3).expect("array header");
        rmp::encode::write_f64(&mut envelope, 1_700_000_000.25).expect("request timestamp");
        rmp::encode::write_bin(&mut envelope, &request_path_hash(path)).expect("path hash");
        rmp::encode::write_bin(&mut envelope, data).expect("request data");
        envelope
    }

    async fn send_request(server: &LinkedServer, envelope: &[u8]) -> RequestId {
        let mut packet = server.outbound.data_packet(envelope).expect("encrypted link packet");
        packet.context = PacketContext::Request;
        let hash = packet.hash().to_bytes();
        let mut request_id = [0u8; crate::hash::ADDRESS_HASH_SIZE];
        request_id.copy_from_slice(&hash[..crate::hash::ADDRESS_HASH_SIZE]);
        super::super::wire::handle_data(
            &packet,
            server.iface,
            server.transport.handler.lock().await,
        )
        .await;
        request_id
    }

    async fn send_ordinary(server: &LinkedServer, data: &[u8]) {
        let packet = server.outbound.data_packet(data).expect("encrypted link packet");
        super::super::wire::handle_data(
            &packet,
            server.iface,
            server.transport.handler.lock().await,
        )
        .await;
    }

    #[tokio::test]
    async fn ordinary_link_ingress_is_authoritative_before_observation_and_proof() {
        let server = linked_server().await;
        let destination_hash = server.destination.lock().await.desc.address_hash;
        let link_id = *server.inbound.lock().await.id();
        let observed = Arc::new(std::sync::Mutex::new(None));
        let observed_callback = Arc::clone(&observed);
        server
            .destination
            .lock()
            .await
            .register_ingress_handler(Arc::new(move |data, context| {
                *observed_callback.lock().unwrap() = Some((data.to_vec(), *context));
                true
            }))
            .unwrap();
        let mut received = server.transport.received_data_events();

        send_ordinary(&server, b"authoritative packet").await;
        let event = timeout(Duration::from_secs(1), received.recv())
            .await
            .expect("accepted ingress observation")
            .expect("received data");
        assert_eq!(event.data.as_slice(), b"authoritative packet");
        assert_eq!(
            *observed.lock().unwrap(),
            Some((
                b"authoritative packet".to_vec(),
                crate::destination::IngressContext {
                    destination: destination_hash,
                    link_id,
                    kind: crate::destination::IngressKind::LinkPacket,
                },
            ))
        );
    }

    #[tokio::test]
    async fn rejected_or_panicking_link_ingress_has_no_observation_or_proof() {
        for handler in [
            Arc::new(|_: &[u8], _: &crate::destination::IngressContext| false)
                as crate::destination::IngressHandler,
            Arc::new(|_: &[u8], _: &crate::destination::IngressContext| -> bool {
                panic!("ingress panic")
            }) as crate::destination::IngressHandler,
        ] {
            let server = linked_server().await;
            server.destination.lock().await.register_ingress_handler(handler).unwrap();
            let mut received = server.transport.received_data_events();
            send_ordinary(&server, b"rejected packet").await;
            assert!(timeout(Duration::from_millis(50), received.recv()).await.is_err());
        }
    }

    async fn next_request_event(
        events: &mut broadcast::Receiver<ServerRequestEvent>,
    ) -> ServerRequestEvent {
        timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("server request outcome timeout")
            .expect("server request outcome channel")
    }

    fn counting_handler(calls: &Arc<AtomicUsize>) -> RequestHandler {
        let calls = Arc::clone(calls);
        Arc::new(move |_, _, _, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            b"response".to_vec()
        })
    }

    #[tokio::test]
    async fn canonical_link_request_uses_destination_registry_and_path_hash() {
        let server = linked_server().await;
        let mut events = server.transport.server_request_events();
        let calls = Arc::new(AtomicUsize::new(0));
        let other_calls = Arc::new(AtomicUsize::new(0));
        let envelope = request_envelope("/page/index.mu", b"hello");
        let path_hash = {
            let mut destination = server.destination.lock().await;
            let path_hash = destination
                .register_request_path(
                    "/page/index.mu",
                    RequestAccess::Public,
                    envelope.len(),
                    64,
                    counting_handler(&calls),
                )
                .expect("request path registration");
            assert_eq!(
                destination.register_request_path(
                    "/page/index.mu",
                    RequestAccess::Public,
                    envelope.len(),
                    64,
                    counting_handler(&calls),
                ),
                Err(RequestRegistrationError::DuplicatePath)
            );
            path_hash
        };

        let mut other = SingleInputDestination::new(
            PrivateIdentity::new_from_rand(OsRng),
            DestinationName::new("nomadnetwork", "other"),
        );
        other
            .register_request_path(
                "/page/index.mu",
                RequestAccess::Public,
                envelope.len(),
                64,
                counting_handler(&other_calls),
            )
            .expect("same path on another destination");
        let other_hash = other.desc.address_hash;
        server
            .transport
            .handler
            .lock()
            .await
            .single_in_destinations
            .insert(other_hash, Arc::new(Mutex::new(other)));
        let request_id = send_request(&server, &envelope).await;
        let event = next_request_event(&mut events).await;

        assert_eq!(event.destination, server.destination.lock().await.desc.address_hash);
        assert_eq!(event.request_id, Some(request_id));
        assert_eq!(event.path_hash, Some(path_hash));
        assert_eq!(event.outcome, ServerRequestOutcome::Handled(b"response".to_vec()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(other_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn link_request_access_uses_only_authenticated_identify_state() {
        let server = linked_server().await;
        let mut events = server.transport.server_request_events();
        let client_identity = PrivateIdentity::new_from_rand(OsRng);
        let client_identity_hash = client_identity.as_identity().address_hash;
        let calls = Arc::new(AtomicUsize::new(0));
        let link_id = *server.outbound.id();
        let max_size = 256;
        {
            let mut destination = server.destination.lock().await;
            destination
                .register_request_path(
                    "/public",
                    RequestAccess::Public,
                    max_size,
                    64,
                    counting_handler(&calls),
                )
                .expect("public path");
            destination
                .register_request_path(
                    "/identified",
                    RequestAccess::Identified,
                    max_size,
                    64,
                    counting_handler(&calls),
                )
                .expect("identified path");
            destination
                .register_request_path(
                    "/allowed",
                    RequestAccess::AllowList(BTreeSet::from([client_identity_hash])),
                    max_size,
                    64,
                    counting_handler(&calls),
                )
                .expect("allow-list path");
            destination
                .register_request_path(
                    "/callback",
                    RequestAccess::Callback(Arc::new(move |remote, context| {
                        context.link_id == link_id
                            && remote.is_some_and(|identity| {
                                identity.address_hash == client_identity_hash
                            })
                    })),
                    max_size,
                    64,
                    counting_handler(&calls),
                )
                .expect("callback path");
        }

        send_request(&server, &request_envelope("/public", b"ok")).await;
        assert!(matches!(
            next_request_event(&mut events).await.outcome,
            ServerRequestOutcome::Handled(_)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        for path in ["/identified", "/allowed", "/callback"] {
            send_request(&server, &request_envelope(path, b"secret")).await;
            assert_eq!(next_request_event(&mut events).await.outcome, ServerRequestOutcome::Denied);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "denials must not invoke handlers");

        let unsigned_identify = vec![0u8; crate::identity::PUBLIC_KEY_LENGTH * 2 + 64];
        for invalid_identify in [&unsigned_identify[..16], unsigned_identify.as_slice()] {
            let mut identify_packet =
                server.outbound.data_packet(invalid_identify).expect("encrypted identify packet");
            identify_packet.context = PacketContext::LinkIdentify;
            server.inbound.lock().await.handle_packet(&identify_packet, server.iface);
        }
        assert!(server.inbound.lock().await.remote_identity().is_none());
        send_request(&server, &request_envelope("/identified", b"secret")).await;
        assert_eq!(next_request_event(&mut events).await.outcome, ServerRequestOutcome::Denied);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let valid_identify =
            server.outbound.identify_packet(&client_identity).expect("signed identify packet");
        server.inbound.lock().await.handle_packet(&valid_identify, server.iface);
        assert_eq!(
            server.inbound.lock().await.remote_identity().map(|identity| identity.address_hash),
            Some(client_identity.as_identity().address_hash)
        );
        for path in ["/identified", "/allowed", "/callback"] {
            send_request(&server, &request_envelope(path, b"secret")).await;
            assert!(matches!(
                next_request_event(&mut events).await.outcome,
                ServerRequestOutcome::Handled(_)
            ));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn malformed_and_oversized_link_requests_do_not_invoke_handler_or_disclose_data() {
        let server = linked_server().await;
        let mut events = server.transport.server_request_events();
        let calls = Arc::new(AtomicUsize::new(0));
        let envelope = request_envelope("/bounded", b"too-large");
        server
            .destination
            .lock()
            .await
            .register_request_path(
                "/bounded",
                RequestAccess::Public,
                envelope.len() - 1,
                64,
                counting_handler(&calls),
            )
            .expect("bounded path");

        send_request(&server, &envelope).await;
        let oversized = next_request_event(&mut events).await;
        assert_eq!(oversized.outcome, ServerRequestOutcome::RequestTooLarge);
        assert!(!matches!(oversized.outcome, ServerRequestOutcome::Handled(_)));

        send_request(&server, &[0x92, 0x00, 0xc0]).await;
        let malformed = next_request_event(&mut events).await;
        assert_eq!(malformed.outcome, ServerRequestOutcome::Malformed);
        assert_eq!(malformed.path_hash, None);
        assert!(!matches!(malformed.outcome, ServerRequestOutcome::Handled(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn observer_lag_cannot_drop_request_dispatch_or_bypass_authorization() {
        const REQUESTS: usize = 40;

        let server = linked_server().await;
        let mut link_observer = server.transport.in_link_events();
        let mut request_observer = server.transport.server_request_events();

        for sequence in 0..REQUESTS {
            let packet = server
                .outbound
                .data_packet(&sequence.to_be_bytes())
                .expect("encrypted observation packet");
            server.inbound.lock().await.handle_packet(&packet, server.iface);
        }
        assert!(matches!(link_observer.try_recv(), Err(broadcast::error::TryRecvError::Lagged(_))));

        let allowed_calls = Arc::new(AtomicUsize::new(0));
        let denied_calls = Arc::new(AtomicUsize::new(0));
        let authorization_checks = Arc::new(AtomicUsize::new(0));
        let checks = Arc::clone(&authorization_checks);
        {
            let mut destination = server.destination.lock().await;
            destination
                .register_request_path(
                    "/allowed-under-load",
                    RequestAccess::Public,
                    256,
                    64,
                    counting_handler(&allowed_calls),
                )
                .expect("allowed path");
            destination
                .register_request_path(
                    "/denied-under-load",
                    RequestAccess::Callback(Arc::new(move |_, _| {
                        checks.fetch_add(1, Ordering::SeqCst);
                        false
                    })),
                    256,
                    64,
                    counting_handler(&denied_calls),
                )
                .expect("denied path");
        }

        for _ in 0..REQUESTS {
            send_request(&server, &request_envelope("/allowed-under-load", b"request")).await;
            send_request(&server, &request_envelope("/denied-under-load", b"request")).await;
        }

        assert_eq!(allowed_calls.load(Ordering::SeqCst), REQUESTS);
        assert_eq!(authorization_checks.load(Ordering::SeqCst), REQUESTS);
        assert_eq!(denied_calls.load(Ordering::SeqCst), 0);
        assert!(matches!(
            request_observer.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(_))
        ));
    }

    #[test]
    fn response_selection_uses_encoded_envelope_size() {
        let request_id = [0x55; crate::hash::ADDRESS_HASH_SIZE];
        let overhead =
            encode_response_envelope(request_id, &[0xc0]).expect("response envelope").len() - 1;
        let packet_response = vec![0; crate::transport::resource::LINK_PACKET_MDU - overhead];
        let resource_response = vec![0; crate::transport::resource::LINK_PACKET_MDU - overhead + 1];

        assert_eq!(
            server_response_mode(
                encode_response_envelope(request_id, &packet_response)
                    .expect("packet envelope")
                    .len(),
                crate::transport::resource::LINK_PACKET_MDU,
            ),
            ServerResponseMode::Packet
        );
        assert_eq!(
            server_response_mode(
                encode_response_envelope(request_id, &resource_response)
                    .expect("resource envelope")
                    .len(),
                crate::transport::resource::LINK_PACKET_MDU,
            ),
            ServerResponseMode::Resource
        );
    }

    #[tokio::test]
    async fn malformed_packet_response_is_bound_to_originating_link() {
        let server = linked_server().await;
        let link_id = *server.outbound.id();
        let request_id = [0x66; crate::hash::ADDRESS_HASH_SIZE];
        {
            let mut handler = server.transport.handler.lock().await;
            handler
                .request_tracker
                .start(
                    request_id,
                    [0x77; crate::hash::ADDRESS_HASH_SIZE],
                    link_id,
                    3,
                    Duration::from_secs(5),
                    64,
                )
                .expect("receipt");
            let mut malformed = Vec::new();
            rmp::encode::write_array_len(&mut malformed, 2).expect("array");
            rmp::encode::write_bin(&mut malformed, &request_id).expect("request id");
            malformed.push(0xd9);
            correlate_packet_response(&mut handler, AddressHash::new([0x99; 16]), &malformed);
            assert_eq!(
                handler.request_tracker.get(&request_id).expect("pending receipt").status,
                crate::transport::request::RequestStatus::Pending
            );
            correlate_packet_response(&mut handler, link_id, &malformed);
            assert_eq!(
                handler.request_tracker.get(&request_id).expect("terminal receipt").status,
                crate::transport::request::RequestStatus::MalformedResponse
            );
        }
    }

    #[tokio::test]
    async fn immediate_packet_and_resource_send_failures_return_terminal_receipts() {
        let server = linked_server().await;
        let link_id = *server.outbound.id();
        let packet = server
            .transport
            .request_over_link(
                &link_id,
                request_path_hash("/packet"),
                &[0xc0],
                Duration::from_secs(5),
                64,
                None,
            )
            .await
            .expect("packet receipt");
        assert_eq!(packet.status, crate::transport::request::RequestStatus::TransportFailed);
        assert!(packet.completed_at.is_some());

        let mut large_data = Vec::new();
        rmp::encode::write_bin(
            &mut large_data,
            &vec![0x51; crate::transport::resource::LINK_PACKET_MDU],
        )
        .expect("large request data");
        let resource = server
            .transport
            .request_over_link(
                &link_id,
                request_path_hash("/resource"),
                &large_data,
                Duration::from_secs(5),
                64,
                None,
            )
            .await
            .expect("resource receipt");
        assert_eq!(resource.status, crate::transport::request::RequestStatus::TransportFailed);
        assert!(resource.request_resource_hash.is_some());
        assert_eq!(server.transport.resource_state_counts().await.total(), 0);
    }

    #[test]
    fn large_request_setup_failure_terminalizes_and_emits_authoritative_receipt() {
        use std::sync::Arc;

        let mut tracker = crate::transport::request::RequestTracker::new(
            4,
            Arc::new(crate::transport::request::SystemRequestClock::new()),
        );
        let request_id = [0x71; crate::hash::ADDRESS_HASH_SIZE];
        tracker
            .start(
                request_id,
                [0x72; crate::hash::ADDRESS_HASH_SIZE],
                AddressHash::new([0x73; 16]),
                crate::transport::resource::LINK_PACKET_MDU + 1,
                Duration::from_secs(5),
                64,
            )
            .expect("request receipt");
        let mut events = tracker.subscribe();

        let receipt = super::links::terminalize_request_setup_failure(&mut tracker, request_id)
            .expect("authoritative terminal receipt");

        assert_eq!(receipt.status, crate::transport::request::RequestStatus::TransportFailed);
        assert!(receipt.completed_at.is_some());
        let event = events.try_recv().expect("terminal event");
        assert_eq!(event.receipt, receipt);
    }

    async fn install_correlated_request_resource(
        server: &LinkedServer,
        request_id: RequestId,
        timeout: Duration,
    ) {
        let link_id = *server.outbound.id();
        let mut handler = server.transport.handler.lock().await;
        handler
            .request_tracker
            .start(request_id, [7; 16], link_id, 32, timeout, 64)
            .expect("request receipt");
        let link = server.inbound.lock().await;
        let (hash, _) = handler
            .resource_manager
            .start_request(&link, vec![0xc0; 512], request_id)
            .expect("request resource");
        drop(link);
        handler.resource_manager.confirm_outbound_dispatch(hash, true);
        assert!(handler.request_tracker.set_request_resource(request_id, hash));
    }

    #[tokio::test]
    async fn cancel_timeout_and_link_close_release_correlated_resources() {
        let cancelled = linked_server().await;
        install_correlated_request_resource(&cancelled, [1; 16], Duration::from_secs(5)).await;
        assert!(cancelled.transport.cancel_request([1; 16]).await);
        assert_eq!(cancelled.transport.resource_state_counts().await.total(), 0);

        let timed_out = linked_server().await;
        install_correlated_request_resource(&timed_out, [2; 16], Duration::ZERO).await;
        assert_eq!(timed_out.transport.poll_request_timeouts().await, 1);
        assert_eq!(timed_out.transport.resource_state_counts().await.total(), 0);

        let closed = linked_server().await;
        install_correlated_request_resource(&closed, [3; 16], Duration::from_secs(5)).await;
        let snapshot = {
            let mut link = closed.inbound.lock().await;
            link.close_with_reason(
                crate::transport::destination_ext::link::LinkCloseReason::Teardown,
            );
            link.state_snapshot()
        };
        closed.transport.handler.lock().await.record_terminal_link(snapshot);
        assert_eq!(closed.transport.resource_state_counts().await.total(), 0);
        assert_eq!(
            closed.transport.request_receipt(&[3; 16]).await.expect("closed receipt").status,
            crate::transport::request::RequestStatus::LinkClosed
        );
    }

    #[tokio::test]
    async fn response_resource_completion_decodes_canonical_envelope_before_observation() {
        let server = linked_server().await;
        let link_id = *server.outbound.id();
        let request_id = [0x31; 16];
        let hash = Hash::new([0x41; 32]);
        let mut generic_resources = server.transport.resource_events();
        {
            let mut handler = server.transport.handler.lock().await;
            handler
                .request_tracker
                .start(request_id, [7; 16], link_id, 3, Duration::from_secs(5), 64)
                .expect("request receipt");
            assert!(handler.request_tracker.resource_advertised(link_id, request_id, hash, 3, 40,));
            let envelope = encode_response_envelope(request_id, &[0xc4, 0x01, 0xaa])
                .expect("response envelope");
            handler
                .publish_resource_events(vec![crate::transport::resource::ResourceEvent {
                    hash,
                    link_id,
                    kind: ResourceEventKind::Complete(
                        crate::transport::resource::ResourceComplete {
                            data: envelope,
                            metadata: None,
                            request_id: Some(request_id),
                            is_request: false,
                            is_response: true,
                            transfer_size: 40,
                            checksum_verified: true,
                        },
                    ),
                }])
                .await;
            let receipt = handler.request_tracker.get(&request_id).expect("completed receipt");
            assert_eq!(receipt.response.as_deref(), Some(&[0xc4, 0x01, 0xaa][..]));
            assert_eq!(receipt.status, crate::transport::request::RequestStatus::Succeeded);
        }
        assert!(matches!(generic_resources.try_recv(), Err(broadcast::error::TryRecvError::Empty)));
    }
}
