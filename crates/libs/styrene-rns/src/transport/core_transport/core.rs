use super::jobs::manage_transport;
use super::*;

impl Transport {
    pub fn new(config: TransportConfig) -> Self {
        Self::new_with_clocks(
            config,
            Arc::new(SystemRequestClock::new()),
            Arc::new(SystemMonotonicClock),
        )
    }

    pub fn new_with_request_clock(config: TransportConfig, clock: Arc<dyn RequestClock>) -> Self {
        Self::new_with_clocks(config, clock, Arc::new(SystemMonotonicClock))
    }

    pub fn new_with_protocol_clock(
        config: TransportConfig,
        protocol_clock: Arc<dyn MonotonicClock>,
    ) -> Self {
        Self::new_with_clocks(config, Arc::new(SystemRequestClock::new()), protocol_clock)
    }

    fn new_with_clocks(
        config: TransportConfig,
        request_clock: Arc<dyn RequestClock>,
        protocol_clock: Arc<dyn MonotonicClock>,
    ) -> Self {
        let (announce_tx, _) = tokio::sync::broadcast::channel(16);
        let (route_tx, _) = tokio::sync::broadcast::channel(64);
        let (link_in_event_tx, _) = tokio::sync::broadcast::channel(16);
        let (link_out_event_tx, _) = tokio::sync::broadcast::channel(16);
        let (received_data_tx, _) = tokio::sync::broadcast::channel(16);
        let (iface_messages_tx, _) = tokio::sync::broadcast::channel(16);
        let (resource_events_tx, _) = tokio::sync::broadcast::channel(16);
        let (server_request_tx, _) = tokio::sync::broadcast::channel(16);

        let path_request_dest = create_path_request_destination().desc.address_hash;
        let iface_manager =
            InterfaceManager::new_with_ingress(config.ingress_queue_capacities, path_request_dest);

        let rx_receiver = iface_manager.receiver();
        let ingress_sender = iface_manager.ingress_sender();

        let iface_manager = Arc::new(Mutex::new(iface_manager));

        let announce_cache_capacity = config.announce_cache_capacity;
        let announce_retry_limit = config.announce_retry_limit;
        let announce_queue_len = config.announce_queue_len;
        let announce_cap = config.announce_cap;
        let path_request_timeout_secs = config.path_request_timeout_secs;
        let link_proof_timeout_secs = config.link_proof_timeout_secs;
        let link_idle_timeout_secs = config.link_idle_timeout_secs;
        let resource_retry_interval_secs = config.resource_retry_interval_secs;
        let resource_retry_limit = config.resource_retry_limit;
        let ratchet_store = config.ratchet_store_path.as_ref().map(|path| {
            let mut store = RatchetStore::new(path.clone());
            store.clean_expired(now_secs());
            store
        });

        let transport_id =
            if config.retransmit { Some(*config.identity.address_hash()) } else { None };
        let path_requests = PathRequests::new(
            config.name.as_str(),
            transport_id,
            announce_queue_len,
            announce_cap,
            path_request_timeout_secs,
        );

        let cancel = CancellationToken::new();
        let name = config.name.clone();
        let handler = Arc::new(Mutex::new(TransportHandler {
            config,
            iface_manager: iface_manager.clone(),
            announce_table: AnnounceTable::new(announce_cache_capacity, announce_retry_limit),
            link_table: LinkTable::new(
                Duration::from_secs(link_proof_timeout_secs),
                Duration::from_secs(link_idle_timeout_secs),
            ),
            path_table: PathTable::new(),
            single_in_destinations: HashMap::new(),
            single_out_destinations: HashMap::new(),
            announce_limits: AnnounceLimits::new(),
            out_links: HashMap::new(),
            in_links: HashMap::new(),
            terminal_link_history: VecDeque::new(),
            packet_cache: Mutex::new(PacketCache::new()),
            path_requests,
            announce_tx,
            route_tx,
            link_in_event_tx: link_in_event_tx.clone(),
            received_data_tx: received_data_tx.clone(),
            ratchet_store,
            resource_manager: ResourceManager::new_with_config_and_clock(
                Duration::from_secs(resource_retry_interval_secs),
                resource_retry_limit,
                protocol_clock.clone(),
            ),
            resource_events_tx: resource_events_tx.clone(),
            server_request_tx: server_request_tx.clone(),
            request_tracker: RequestTracker::new(
                crate::transport::request::DEFAULT_REQUEST_RECEIPT_CAPACITY,
                request_clock,
            ),
            protocol_clock: protocol_clock.clone(),
            fixed_dest_path_requests: path_request_dest,
            cancel: cancel.clone(),
            receipt_handler: None,
        }));

        let weak_handler = Arc::downgrade(&handler);
        ingress_sender.set_admission(move |message| {
            let weak_handler = weak_handler.clone();
            Box::pin(async move {
                let address = message.address;
                let message = match message.admit() {
                    Ok(message) => message,
                    Err(_) => {
                        let Some(handler) = weak_handler.upgrade() else {
                            return Err(crate::transport::iface::InterfaceRxSendError);
                        };
                        handler.lock().await.iface_manager.lock().await.record_drop(
                            &address,
                            crate::transport::iface::InterfaceDropReason::MalformedFrame,
                        );
                        return Ok(None);
                    }
                };
                let Some(handler) = weak_handler.upgrade() else {
                    return Err(crate::transport::iface::InterfaceRxSendError);
                };
                if let Some(reason) =
                    super::jobs::protocol_drop_reason(&message.packet, &handler, address).await
                {
                    handler.lock().await.iface_manager.lock().await.record_drop(&address, reason);
                    return Ok(None);
                }
                Ok(Some(message))
            })
        });

        let manager_task = {
            let handler = handler.clone();
            tokio::spawn(manage_transport(handler, rx_receiver, iface_messages_tx.clone()))
        };
        fn spawn_link_data_forwarder(
            mut link_rx: broadcast::Receiver<LinkEventData>,
            received_data_tx: broadcast::Sender<ReceivedData>,
        ) {
            tokio::spawn(async move {
                loop {
                    match link_rx.recv().await {
                        Ok(event) => {
                            if let LinkEvent::Data(ref payload) = event.event {
                                let _ = received_data_tx.send(ReceivedData {
                                    destination: event.address_hash,
                                    link_id: Some(event.id),
                                    data: PacketDataBuffer::new_from_slice(payload.as_slice()),
                                    payload_mode: ReceivedPayloadMode::FullWire,
                                    ratchet_used: false,
                                    context: Some(payload.context()),
                                    request_id: payload.request_id(),
                                    hops: None,
                                    interface: None,
                                });
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    }
                }
            });
        }
        {
            spawn_link_data_forwarder(link_in_event_tx.subscribe(), received_data_tx.clone());
            spawn_link_data_forwarder(link_out_event_tx.subscribe(), received_data_tx.clone());
        }

        Self {
            name,
            iface_manager,
            link_in_event_tx,
            link_out_event_tx,
            received_data_tx,
            iface_messages_tx,
            resource_events_tx,
            server_request_tx,
            handler,
            cancel,
            manager_task: StdMutex::new(Some(manager_task)),
        }
    }

    pub async fn outbound(&self, packet: &Packet) {
        let destination = packet.destination;
        let (packet, maybe_iface) = self.handler.lock().await.path_table.handle_packet(packet);

        if let Some(iface) = maybe_iface {
            let routed = packet.header.header_type == HeaderType::Type2;
            let dispatch = self.send_direct(iface, packet).await;
            if routed && dispatch.sent_ifaces > 0 {
                self.handler.lock().await.path_table.refresh(&destination);
            }
            log::trace!("Sent outbound packet to {}", iface);
        }
        if maybe_iface.is_none() {
            let handler = self.handler.lock().await;
            if handler.config.broadcast {
                handler.send(TxMessage { tx_type: TxMessageType::Broadcast(None), packet }).await;
            } else {
                log::trace!(
                    "tp({}): no route for outbound packet dst={}",
                    self.name,
                    packet.destination
                );
            }
        }
    }

    /// Query path table entry for a destination.
    pub async fn path_info(&self, dest: &AddressHash) -> Option<(u8, AddressHash)> {
        let handler = self.handler.lock().await;
        handler.path_table.get(dest).map(|e| (e.hops, e.iface))
    }

    /// Dump the entire path table as (destination, hops, received_from, interface) tuples.
    pub async fn path_table_entries(&self) -> Vec<(AddressHash, u8, AddressHash, AddressHash)> {
        let handler = self.handler.lock().await;
        handler
            .path_table
            .entries()
            .map(|(dest, entry)| (*dest, entry.hops, entry.received_from, entry.iface))
            .collect()
    }

    pub async fn path_snapshot(
        &self,
        dest: &AddressHash,
    ) -> Option<crate::transport::core_transport::path_table::PathSnapshot> {
        let handler = self.handler.lock().await;
        handler.path_table.snapshot(dest, std::time::Instant::now())
    }

    pub async fn path_snapshots(
        &self,
    ) -> Vec<crate::transport::core_transport::path_table::PathSnapshot> {
        let handler = self.handler.lock().await;
        handler.path_table.snapshots(std::time::Instant::now())
    }

    pub fn iface_manager(&self) -> Arc<Mutex<InterfaceManager>> {
        self.iface_manager.clone()
    }

    /// Return per-interface byte counter snapshots (tx_bytes, rx_bytes).
    pub async fn interface_stats(
        &self,
    ) -> std::collections::HashMap<AddressHash, crate::transport::iface::InterfaceStatsSnapshot>
    {
        self.iface_manager.lock().await.interface_stats()
    }

    pub async fn ingress_snapshot(&self) -> IngressSnapshot {
        self.iface_manager.lock().await.ingress_snapshot()
    }

    pub async fn interface_snapshots(&self) -> Vec<crate::transport::iface::InterfaceSnapshot> {
        self.iface_manager.lock().await.interface_snapshots()
    }

    pub fn iface_rx(&self) -> broadcast::Receiver<RxMessage> {
        self.iface_messages_tx.subscribe()
    }

    pub fn resource_events(&self) -> broadcast::Receiver<ResourceEvent> {
        self.resource_events_tx.subscribe()
    }

    pub async fn recv_announces(&self) -> broadcast::Receiver<AnnounceEvent> {
        self.handler.lock().await.announce_tx.subscribe()
    }

    pub async fn route_events(&self) -> broadcast::Receiver<path_table::RouteEvent> {
        self.handler.lock().await.route_tx.subscribe()
    }

    pub async fn send_packet(&self, packet: Packet) {
        let mut handler = self.handler.lock().await;
        handler.send_packet(packet).await;
    }

    pub async fn send_packet_with_outcome(&self, packet: Packet) -> SendPacketOutcome {
        let mut handler = self.handler.lock().await;
        handler.send_packet_with_outcome(packet).await
    }

    pub async fn send_packet_with_trace(&self, packet: Packet) -> SendPacketTrace {
        let mut handler = self.handler.lock().await;
        handler.send_packet_with_trace(packet).await
    }

    pub async fn send_announce(
        &self,
        destination: &Arc<Mutex<SingleInputDestination>>,
        app_data: Option<&[u8]>,
    ) -> SendPacketOutcome {
        let mut destination = destination.lock().await;
        crate::transport_diagnostic!(
            "[tp] announce_tx dst={} app_data_len={}",
            destination.desc.address_hash,
            app_data.map(|value| value.len()).unwrap_or(0)
        );
        let packet = destination.announce(OsRng, app_data).expect("valid announce packet");
        let mut handler = self.handler.lock().await;
        handler.send_packet_with_outcome(packet).await
    }

    pub async fn set_receipt_handler(&mut self, handler: Box<dyn ReceiptHandler>) {
        self.handler.lock().await.receipt_handler = Some(Arc::from(handler));
    }

    pub fn emit_receipt_for_test(&self, receipt: DeliveryReceipt) {
        let receipt_handler =
            self.handler.try_lock().ok().and_then(|handler| handler.receipt_handler.clone());

        if let Some(handler) = receipt_handler {
            handler.on_receipt(&receipt);
        }
    }

    pub async fn handle_inbound_for_test(&self, packet: Packet) {
        let (receipt, receipt_handler) = {
            let handler = self.handler.lock().await;
            let receipt = super::wire::validated_receipt_hash(&packet, &handler)
                .await
                .map(DeliveryReceipt::new);
            let receipt_handler = handler.receipt_handler.clone();
            (receipt, receipt_handler)
        };

        if let (Some(receipt), Some(handler)) = (receipt, receipt_handler) {
            handler.on_receipt(&receipt);
        }
    }

    pub async fn send_broadcast(&self, packet: Packet, from_iface: Option<AddressHash>) {
        self.handler
            .lock()
            .await
            .send(TxMessage { tx_type: TxMessageType::Broadcast(from_iface), packet })
            .await;
    }

    pub async fn send_direct(&self, addr: AddressHash, packet: Packet) -> TxDispatchTrace {
        self.handler
            .lock()
            .await
            .send(TxMessage { tx_type: TxMessageType::Direct(addr), packet })
            .await
    }
}
