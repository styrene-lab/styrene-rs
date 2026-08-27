//! Route worker bridging authoritative RNS path-table transitions into IPC events.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rns_core::transport::core_transport::path_table::{
    RouteEvent, RouteEventKind as RnsRouteEventKind, RouteLossReason as RnsRouteLossReason,
};
use styrene_ipc::types::{
    ObservationMetadata, ObservationSource, PathInfo, RouteEventInfo, RouteEventKind,
    RouteLossReason,
};
use tokio::task::JoinHandle;

use crate::services::EventService;
use crate::transport::mesh_transport::MeshTransport;

const PATH_FRESHNESS_THRESHOLD_SECS: u64 = 300;

fn unix_time(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
}

fn map_route_event(event: RouteEvent) -> RouteEventInfo {
    let mut route = PathInfo::default();
    route.destination_hash = hex::encode(event.route.destination.as_slice());
    route.hops = Some(u32::from(event.route.hops));
    route.next_hop = Some(hex::encode(event.route.received_from.as_slice()));
    route.interface = Some(hex::encode(event.route.iface.as_slice()));
    route.expires = unix_time(event.route.expires_at);
    route.observation.source = ObservationSource::TransportPathTable;
    route.observation.observed_at = unix_time(event.route.observed_at);
    route.observation.age_secs = Some(event.route.age.as_secs());
    route.observation.freshness_threshold_secs = Some(PATH_FRESHNESS_THRESHOLD_SECS);
    route.observation.stale = event.route.age.as_secs() > PATH_FRESHNESS_THRESHOLD_SECS;

    let occurred_at = unix_time(event.occurred_at);
    let mut observation = ObservationMetadata::default();
    observation.source = ObservationSource::TransportPathTable;
    observation.observed_at = occurred_at;
    observation.age_secs = occurred_at.map(|_| 0);
    observation.freshness_threshold_secs = Some(PATH_FRESHNESS_THRESHOLD_SECS);

    let mut info = RouteEventInfo::default();
    info.kind = match event.kind {
        RnsRouteEventKind::Discovered => RouteEventKind::Discovered,
        RnsRouteEventKind::Lost => RouteEventKind::Lost,
        RnsRouteEventKind::Rediscovered => RouteEventKind::Rediscovered,
    };
    info.route = route;
    info.loss_reason = event.loss_reason.map(|reason| match reason {
        RnsRouteLossReason::Expired => RouteLossReason::Expired,
        RnsRouteLossReason::InterfaceUnavailable => RouteLossReason::InterfaceUnavailable,
    });
    info.observation = observation;
    info
}

pub fn spawn_route_worker(
    transport: Arc<dyn MeshTransport>,
    events: Arc<EventService>,
) -> JoinHandle<()> {
    let mut rx = transport.subscribe_routes();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => events.emit_route_event(map_route_event(event)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    crate::daemon_diagnostic!("[route-worker] lagged, skipped {count} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use rns_core::hash::AddressHash;
    use rns_core::transport::core_transport::path_table::PathSnapshot;

    use crate::transport::mock_transport::MockTransport;

    #[tokio::test]
    async fn route_loss_reaches_event_service_with_final_snapshot() {
        let transport = Arc::new(MockTransport::new_default());
        let events = Arc::new(EventService::new());
        let mut receiver = events.subscribe_routes();
        let _worker = spawn_route_worker(transport.clone(), events);
        let hash = AddressHash::new([7; 16]);
        let observed_at = UNIX_EPOCH + Duration::from_secs(100);
        transport.inject_route(RouteEvent {
            kind: RnsRouteEventKind::Lost,
            route: PathSnapshot {
                destination: hash,
                hops: 2,
                received_from: hash,
                iface: hash,
                age: Duration::from_secs(301),
                observed_at,
                lifetime: Duration::from_secs(600),
                expires_at: observed_at + Duration::from_secs(600),
            },
            loss_reason: Some(RnsRouteLossReason::Expired),
            occurred_at: SystemTime::now(),
        });

        let event = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("route event timeout")
            .expect("route channel closed");
        match event {
            styrene_ipc::types::DaemonEvent::Route { event } => {
                assert_eq!(event.kind, RouteEventKind::Lost);
                assert_eq!(event.loss_reason, Some(RouteLossReason::Expired));
                assert_eq!(event.route.observation.age_secs, Some(301));
                assert!(event.route.observation.stale);
            }
            other => panic!("expected route event, got {other:?}"),
        }
    }
}
