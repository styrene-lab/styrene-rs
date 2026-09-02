use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;

use crate::destination::{RequestId, RequestPathHash};
use crate::hash::{AddressHash, Hash};

pub fn encode_response_envelope(request_id: RequestId, response: &[u8]) -> Option<Vec<u8>> {
    let mut envelope = Vec::new();
    rmp::encode::write_array_len(&mut envelope, 2).ok()?;
    rmp::encode::write_bin(&mut envelope, &request_id).ok()?;
    envelope.extend_from_slice(response);
    Some(envelope)
}

/// Encode the raw bytes of a metadata-bearing response resource as a MessagePack
/// binary value. Python `Link.response_resource_concluded` hands such responses
/// (NomadNet file downloads) to the requester as `bytes` without the
/// `[request_id, response]` envelope, so the receipt keeps the same shape as an
/// enveloped binary response.
pub fn encode_raw_response(data: &[u8]) -> Option<Vec<u8>> {
    let mut encoded = Vec::with_capacity(data.len() + 5);
    rmp::encode::write_bin(&mut encoded, data).ok()?;
    Some(encoded)
}

pub fn decode_response_envelope(payload: &[u8]) -> Result<(RequestId, Vec<u8>), Option<RequestId>> {
    use std::io::Cursor;

    let mut cursor = Cursor::new(payload);
    if rmp::decode::read_array_len(&mut cursor).ok() != Some(2) {
        return Err(None);
    }
    let Ok(rmpv::Value::Binary(id)) = rmpv::decode::read_value(&mut cursor) else {
        return Err(None);
    };
    let Ok(request_id) = id.try_into() else { return Err(None) };
    let data_start = usize::try_from(cursor.position()).map_err(|_| Some(request_id))?;
    rmpv::decode::read_value(&mut cursor).map_err(|_| Some(request_id))?;
    if usize::try_from(cursor.position()).ok() != Some(payload.len()) {
        return Err(Some(request_id));
    }
    Ok((request_id, payload[data_start..].to_vec()))
}

pub const DEFAULT_REQUEST_RECEIPT_CAPACITY: usize = 256;

pub fn encode_request_envelope(
    requested_at: f64,
    path_hash: RequestPathHash,
    data: &[u8],
) -> Option<Vec<u8>> {
    let mut envelope = Vec::new();
    rmp::encode::write_array_len(&mut envelope, 3).ok()?;
    rmp::encode::write_f64(&mut envelope, requested_at).ok()?;
    rmp::encode::write_bin(&mut envelope, &path_hash).ok()?;
    envelope.extend_from_slice(data);
    Some(envelope)
}

pub fn canonical_request_id(packed_request: &[u8]) -> RequestId {
    crate::hash::address_hash(packed_request)
}

pub trait RequestClock: Send + Sync {
    fn now(&self) -> Duration;
}

#[derive(Debug)]
pub struct SystemRequestClock {
    origin: Instant,
}

impl SystemRequestClock {
    pub fn new() -> Self {
        Self { origin: Instant::now() }
    }
}

impl Default for SystemRequestClock {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestClock for SystemRequestClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStatus {
    Pending,
    Receiving,
    Succeeded,
    LinkClosed,
    TimedOut,
    MalformedResponse,
    Cancelled,
    ResponseTooLarge,
    ResourceFailed,
    TransportFailed,
}

impl RequestStatus {
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending | Self::Receiving)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestProtocolError {
    LinkClosed,
    Timeout,
    MalformedResponse,
    Cancelled,
    ResponseTooLarge,
    ResourceFailed,
    TransportFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseTransfer {
    None,
    Packet,
    Resource { hash: Hash },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestReceipt {
    pub request_id: RequestId,
    pub path_hash: RequestPathHash,
    pub link_id: AddressHash,
    pub started_at: Duration,
    pub deadline: Duration,
    pub request_size: usize,
    pub response_size: Option<usize>,
    pub response_transfer_size: Option<u64>,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub progress: f32,
    pub response_transfer: ResponseTransfer,
    pub response: Option<Vec<u8>>,
    /// Packed MessagePack metadata carried by a metadata-bearing response
    /// resource (Python file responses). `None` for enveloped responses.
    pub response_metadata: Option<Vec<u8>>,
    pub status: RequestStatus,
    pub protocol_error: Option<RequestProtocolError>,
    pub completed_at: Option<Duration>,
    pub rtt: Option<Duration>,
    pub request_resource_hash: Option<Hash>,
    /// Higher-level operation correlation retained across packet/resource observations.
    pub correlation_id: Option<String>,
    max_response_size: usize,
}

impl RequestReceipt {
    pub(crate) const fn maximum_response_size(&self) -> usize {
        self.max_response_size
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestObservation {
    pub receipt: RequestReceipt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStartError {
    Capacity,
    Duplicate,
    InvalidLimit,
}

#[derive(Debug, Clone)]
pub struct RequestOptions {
    pub timeout: Duration,
    pub max_response_size: usize,
    pub correlation_id: Option<String>,
}

pub struct RequestTracker {
    receipts: HashMap<RequestId, RequestReceipt>,
    order: VecDeque<RequestId>,
    resources: HashMap<Hash, RequestId>,
    capacity: usize,
    clock: Arc<dyn RequestClock>,
    events: broadcast::Sender<RequestObservation>,
}

impl RequestTracker {
    pub fn new(capacity: usize, clock: Arc<dyn RequestClock>) -> Self {
        let (events, _) = broadcast::channel(capacity.max(1));
        Self {
            receipts: HashMap::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            resources: HashMap::new(),
            capacity,
            clock,
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RequestObservation> {
        self.events.subscribe()
    }

    pub fn get(&self, request_id: &RequestId) -> Option<&RequestReceipt> {
        self.receipts.get(request_id)
    }

    pub fn start(
        &mut self,
        request_id: RequestId,
        path_hash: RequestPathHash,
        link_id: AddressHash,
        request_size: usize,
        timeout: Duration,
        max_response_size: usize,
    ) -> Result<RequestReceipt, RequestStartError> {
        self.start_correlated(
            request_id,
            path_hash,
            link_id,
            request_size,
            RequestOptions { timeout, max_response_size, correlation_id: None },
        )
    }

    pub fn start_correlated(
        &mut self,
        request_id: RequestId,
        path_hash: RequestPathHash,
        link_id: AddressHash,
        request_size: usize,
        options: RequestOptions,
    ) -> Result<RequestReceipt, RequestStartError> {
        if options.max_response_size == 0 {
            return Err(RequestStartError::InvalidLimit);
        }
        if self.receipts.contains_key(&request_id) {
            return Err(RequestStartError::Duplicate);
        }
        self.make_room()?;

        let started_at = self.clock.now();
        let receipt = RequestReceipt {
            request_id,
            path_hash,
            link_id,
            started_at,
            deadline: started_at.saturating_add(options.timeout),
            request_size,
            response_size: None,
            response_transfer_size: None,
            received_bytes: 0,
            total_bytes: 0,
            progress: 0.0,
            response_transfer: ResponseTransfer::None,
            response: None,
            response_metadata: None,
            status: RequestStatus::Pending,
            protocol_error: None,
            completed_at: None,
            rtt: None,
            request_resource_hash: None,
            correlation_id: options.correlation_id,
            max_response_size: options.max_response_size,
        };
        self.order.push_back(request_id);
        self.receipts.insert(request_id, receipt.clone());
        self.emit(&receipt);
        Ok(receipt)
    }

    pub fn snapshot(&self) -> Vec<RequestReceipt> {
        self.order.iter().filter_map(|id| self.receipts.get(id).cloned()).collect()
    }

    pub fn set_request_resource(&mut self, request_id: RequestId, hash: Hash) -> bool {
        let Some(receipt) = self.receipts.get_mut(&request_id) else { return false };
        if receipt.status.is_terminal() || receipt.request_resource_hash.is_some() {
            return false;
        }
        receipt.request_resource_hash = Some(hash);
        let observation = receipt.clone();
        self.emit(&observation);
        true
    }

    fn make_room(&mut self) -> Result<(), RequestStartError> {
        while self.receipts.len() >= self.capacity {
            let Some(index) = self.order.iter().position(|id| {
                self.receipts.get(id).is_some_and(|receipt| receipt.status.is_terminal())
            }) else {
                return Err(RequestStartError::Capacity);
            };
            let Some(request_id) = self.order.remove(index) else {
                return Err(RequestStartError::Capacity);
            };
            self.receipts.remove(&request_id);
            self.resources.retain(|_, correlated| *correlated != request_id);
        }
        Ok(())
    }

    pub fn packet_response(
        &mut self,
        link_id: AddressHash,
        request_id: RequestId,
        response: Vec<u8>,
        transfer_size: u64,
    ) -> bool {
        let Some(receipt) = self.receipts.get_mut(&request_id) else { return false };
        if receipt.status.is_terminal()
            || receipt.link_id != link_id
            || receipt.response_transfer != ResponseTransfer::None
        {
            return false;
        }
        if response.len() > receipt.max_response_size {
            return self.finish(
                request_id,
                RequestStatus::ResponseTooLarge,
                Some(RequestProtocolError::ResponseTooLarge),
            );
        }
        receipt.response_size = Some(response.len());
        receipt.response_transfer_size = Some(transfer_size);
        receipt.received_bytes = transfer_size;
        receipt.total_bytes = transfer_size;
        receipt.progress = 1.0;
        receipt.response_transfer = ResponseTransfer::Packet;
        receipt.response = Some(response);
        self.finish(request_id, RequestStatus::Succeeded, None)
    }

    pub fn resource_advertised(
        &mut self,
        link_id: AddressHash,
        request_id: RequestId,
        hash: Hash,
        response_size: usize,
        transfer_size: u64,
    ) -> bool {
        let Some(receipt) = self.receipts.get_mut(&request_id) else { return false };
        if receipt.status.is_terminal()
            || receipt.link_id != link_id
            || receipt.response_transfer != ResponseTransfer::None
        {
            return false;
        }
        if response_size > receipt.max_response_size {
            self.finish(
                request_id,
                RequestStatus::ResponseTooLarge,
                Some(RequestProtocolError::ResponseTooLarge),
            );
            return false;
        }
        receipt.status = RequestStatus::Receiving;
        receipt.response_size = Some(response_size);
        receipt.total_bytes = transfer_size;
        receipt.response_transfer = ResponseTransfer::Resource { hash };
        self.resources.insert(hash, request_id);
        let observation = receipt.clone();
        self.emit(&observation);
        true
    }

    pub fn resource_progress(
        &mut self,
        link_id: AddressHash,
        hash: Hash,
        received_bytes: u64,
        total_bytes: u64,
    ) -> bool {
        let Some(request_id) = self.resources.get(&hash).copied() else { return false };
        let Some(receipt) = self.receipts.get_mut(&request_id) else { return false };
        if receipt.status.is_terminal() || receipt.link_id != link_id {
            return false;
        }
        receipt.progress = if total_bytes == 0 {
            0.0
        } else {
            (received_bytes as f64 / total_bytes as f64).clamp(0.0, 1.0) as f32
        };
        receipt.received_bytes = received_bytes;
        receipt.total_bytes = total_bytes;
        receipt.response_transfer_size = Some(received_bytes);
        let observation = receipt.clone();
        self.emit(&observation);
        true
    }

    pub fn resource_complete(
        &mut self,
        link_id: AddressHash,
        hash: Hash,
        response: Vec<u8>,
        metadata: Option<Vec<u8>>,
        transfer_size: u64,
    ) -> bool {
        let Some(request_id) = self.resources.get(&hash).copied() else { return false };
        let Some(receipt) = self.receipts.get_mut(&request_id) else { return false };
        if receipt.status.is_terminal()
            || receipt.link_id != link_id
            || receipt.response_transfer != (ResponseTransfer::Resource { hash })
        {
            return false;
        }
        if response.len() > receipt.max_response_size {
            return self.finish(
                request_id,
                RequestStatus::ResponseTooLarge,
                Some(RequestProtocolError::ResponseTooLarge),
            );
        }
        receipt.response_size = Some(response.len());
        receipt.response_transfer_size = Some(transfer_size);
        receipt.received_bytes = transfer_size;
        receipt.total_bytes = transfer_size;
        receipt.progress = 1.0;
        receipt.response = Some(response);
        receipt.response_metadata = metadata;
        self.finish(request_id, RequestStatus::Succeeded, None)
    }

    pub fn malformed(&mut self, link_id: AddressHash, request_id: RequestId) -> bool {
        if !self.receipts.get(&request_id).is_some_and(|receipt| receipt.link_id == link_id) {
            return false;
        }
        self.finish(
            request_id,
            RequestStatus::MalformedResponse,
            Some(RequestProtocolError::MalformedResponse),
        )
    }

    pub fn cancel(&mut self, request_id: RequestId) -> bool {
        self.finish(request_id, RequestStatus::Cancelled, Some(RequestProtocolError::Cancelled))
    }

    pub fn transport_failed(&mut self, request_id: RequestId) -> bool {
        self.finish(
            request_id,
            RequestStatus::TransportFailed,
            Some(RequestProtocolError::TransportFailed),
        )
    }

    pub fn resource_failed(&mut self, link_id: AddressHash, hash: Hash) -> bool {
        let Some(request_id) = self.resources.get(&hash).copied() else { return false };
        if !self.receipts.get(&request_id).is_some_and(|receipt| receipt.link_id == link_id) {
            return false;
        }
        self.finish(
            request_id,
            RequestStatus::ResourceFailed,
            Some(RequestProtocolError::ResourceFailed),
        )
    }

    pub fn correlated_resources(&self, request_id: RequestId) -> Vec<Hash> {
        let Some(receipt) = self.receipts.get(&request_id) else { return Vec::new() };
        let mut hashes = Vec::with_capacity(2);
        if let Some(hash) = receipt.request_resource_hash {
            hashes.push(hash);
        }
        if let ResponseTransfer::Resource { hash } = receipt.response_transfer
            && !hashes.contains(&hash)
        {
            hashes.push(hash);
        }
        hashes
    }

    pub fn response_resource_request_id(&self, hash: Hash) -> Option<RequestId> {
        self.resources.get(&hash).copied()
    }

    pub fn request_ids_by_correlation(&self, correlation_id: &str) -> Vec<RequestId> {
        self.receipts
            .values()
            .filter(|receipt| {
                !receipt.status.is_terminal()
                    && receipt.correlation_id.as_deref() == Some(correlation_id)
            })
            .map(|receipt| receipt.request_id)
            .collect()
    }

    pub fn link_closed(&mut self, link_id: AddressHash) -> usize {
        let ids = self
            .receipts
            .values()
            .filter(|receipt| receipt.link_id == link_id && !receipt.status.is_terminal())
            .map(|receipt| receipt.request_id)
            .collect::<Vec<_>>();
        for request_id in &ids {
            self.finish(
                *request_id,
                RequestStatus::LinkClosed,
                Some(RequestProtocolError::LinkClosed),
            );
        }
        ids.len()
    }

    pub fn poll_timeouts(&mut self) -> usize {
        let ids = self.timeout_due_ids();
        for request_id in &ids {
            self.timeout(*request_id);
        }
        ids.len()
    }

    pub fn timeout_due_ids(&self) -> Vec<RequestId> {
        let now = self.clock.now();
        self.receipts
            .values()
            .filter(|receipt| !receipt.status.is_terminal() && now >= receipt.deadline)
            .map(|receipt| receipt.request_id)
            .collect()
    }

    pub fn timeout(&mut self, request_id: RequestId) -> bool {
        self.finish(request_id, RequestStatus::TimedOut, Some(RequestProtocolError::Timeout))
    }

    fn finish(
        &mut self,
        request_id: RequestId,
        status: RequestStatus,
        protocol_error: Option<RequestProtocolError>,
    ) -> bool {
        let now = self.clock.now();
        let Some(receipt) = self.receipts.get_mut(&request_id) else { return false };
        if receipt.status.is_terminal() {
            return false;
        }
        receipt.status = status;
        receipt.protocol_error = protocol_error;
        receipt.completed_at = Some(now);
        receipt.rtt = Some(now.saturating_sub(receipt.started_at));
        let observation = receipt.clone();
        self.emit(&observation);
        true
    }

    fn emit(&self, receipt: &RequestReceipt) {
        let _ = self.events.send(RequestObservation { receipt: receipt.clone() });
    }
}

impl Default for RequestTracker {
    fn default() -> Self {
        Self::new(DEFAULT_REQUEST_RECEIPT_CAPACITY, Arc::new(SystemRequestClock::new()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use sha2::Digest as _;

    use super::*;

    #[derive(Default)]
    struct ManualClock(AtomicU64);

    impl ManualClock {
        fn advance(&self, duration: Duration) {
            self.0.fetch_add(duration.as_millis() as u64, Ordering::SeqCst);
        }
    }

    impl RequestClock for ManualClock {
        fn now(&self) -> Duration {
            Duration::from_millis(self.0.load(Ordering::SeqCst))
        }
    }

    fn start(tracker: &mut RequestTracker, id: u8, link: u8, max: usize) {
        tracker
            .start([id; 16], [7; 16], AddressHash::new([link; 16]), 23, Duration::from_secs(5), max)
            .expect("request starts");
    }

    #[test]
    fn receipt_exists_before_any_response_and_terminal_state_is_monotonic() {
        let clock = Arc::new(ManualClock::default());
        let mut tracker = RequestTracker::new(4, clock);
        start(&mut tracker, 1, 2, 16);

        let pending = tracker.get(&[1; 16]).expect("receipt exists");
        assert_eq!(pending.status, RequestStatus::Pending);
        assert_eq!(pending.request_size, 23);
        assert_eq!(pending.deadline, Duration::from_secs(5));

        assert!(tracker.packet_response(AddressHash::new([2; 16]), [1; 16], b"ok".to_vec(), 2));
        assert!(!tracker.cancel([1; 16]));
        assert_eq!(
            tracker.get(&[1; 16]).expect("receipt retained").status,
            RequestStatus::Succeeded
        );
    }

    #[test]
    fn timeout_and_cancellation_use_injected_monotonic_time() {
        let clock = Arc::new(ManualClock::default());
        let mut tracker = RequestTracker::new(4, clock.clone());
        start(&mut tracker, 1, 2, 16);
        start(&mut tracker, 2, 2, 16);
        assert!(tracker.cancel([2; 16]));

        clock.advance(Duration::from_secs(5));
        assert_eq!(tracker.poll_timeouts(), 1);
        assert_eq!(tracker.get(&[1; 16]).expect("timed out").status, RequestStatus::TimedOut);
        assert_eq!(tracker.get(&[2; 16]).expect("cancelled").status, RequestStatus::Cancelled);
    }

    #[test]
    fn packet_and_resource_responses_enforce_maximum_size() {
        let clock = Arc::new(ManualClock::default());
        let mut tracker = RequestTracker::new(4, clock);
        start(&mut tracker, 1, 2, 3);
        assert!(tracker.packet_response(AddressHash::new([2; 16]), [1; 16], vec![0; 4], 4));
        assert_eq!(tracker.get(&[1; 16]).expect("receipt").status, RequestStatus::ResponseTooLarge);

        start(&mut tracker, 2, 2, 3);
        let hash = Hash::new([9; 32]);
        assert!(tracker.resource_advertised(AddressHash::new([2; 16]), [2; 16], hash, 3, 3));
        assert!(tracker.resource_progress(AddressHash::new([2; 16]), hash, 2, 3));
        assert!(tracker.resource_complete(AddressHash::new([2; 16]), hash, vec![1, 2, 3], None, 3));
        let receipt = tracker.get(&[2; 16]).expect("resource receipt");
        assert_eq!(receipt.status, RequestStatus::Succeeded);
        assert_eq!(receipt.progress, 1.0);
        assert_eq!(receipt.response.as_deref(), Some(&[1, 2, 3][..]));
    }

    #[test]
    fn link_close_finishes_only_matching_pending_receipts() {
        let clock = Arc::new(ManualClock::default());
        let mut tracker = RequestTracker::new(4, clock);
        start(&mut tracker, 1, 2, 16);
        start(&mut tracker, 2, 3, 16);
        assert_eq!(tracker.link_closed(AddressHash::new([2; 16])), 1);
        assert_eq!(tracker.get(&[1; 16]).expect("closed").status, RequestStatus::LinkClosed);
        assert_eq!(tracker.get(&[2; 16]).expect("pending").status, RequestStatus::Pending);
    }

    #[test]
    fn capacity_evicts_terminal_receipts_but_never_pending_state() {
        let clock = Arc::new(ManualClock::default());
        let mut tracker = RequestTracker::new(2, clock);
        start(&mut tracker, 1, 2, 16);
        start(&mut tracker, 2, 2, 16);
        assert_eq!(
            tracker.start(
                [3; 16],
                [7; 16],
                AddressHash::new([2; 16]),
                1,
                Duration::from_secs(1),
                1,
            ),
            Err(RequestStartError::Capacity)
        );
        assert!(tracker.cancel([1; 16]));
        start(&mut tracker, 3, 2, 16);
        assert!(tracker.get(&[1; 16]).is_none());
    }

    #[test]
    fn canonical_response_envelope_roundtrips_application_value_including_nil() {
        let request_id = [0x44; 16];
        for response in [vec![0xc0], vec![0xc4, 0x02, 0xaa, 0xbb]] {
            let packed =
                encode_response_envelope(request_id, &response).expect("response envelope");
            assert_eq!(decode_response_envelope(&packed), Ok((request_id, response)));
        }
    }

    #[test]
    fn large_request_id_is_hash_of_canonical_packed_request() {
        let path_hash = [0x22; 16];
        let packed = encode_request_envelope(1_700_000_000.25, path_hash, &[0xc0])
            .expect("request envelope");
        let digest = sha2::Sha256::digest(&packed);
        assert_eq!(canonical_request_id(&packed).as_slice(), &digest[..16]);
        let mut cursor = std::io::Cursor::new(&packed);
        assert_eq!(rmp::decode::read_array_len(&mut cursor).expect("array length"), 3);
    }

    #[test]
    fn only_one_response_resource_is_accepted_and_snapshot_retains_terminal_metrics() {
        let clock = Arc::new(ManualClock::default());
        let mut tracker = RequestTracker::new(4, clock.clone());
        start(&mut tracker, 1, 2, 64);
        let first = Hash::new([1; 32]);
        let extra = Hash::new([2; 32]);
        let link = AddressHash::new([2; 16]);
        assert!(tracker.resource_advertised(link, [1; 16], first, 3, 40));
        assert!(!tracker.resource_advertised(link, [1; 16], extra, 3, 40));
        clock.advance(Duration::from_millis(25));
        assert!(tracker.resource_complete(link, first, vec![1, 2, 3], None, 40));

        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].completed_at, Some(Duration::from_millis(25)));
        assert_eq!(snapshot[0].rtt, Some(Duration::from_millis(25)));
        assert_eq!(snapshot[0].response_transfer_size, Some(40));
        assert_eq!(snapshot[0].received_bytes, 40);
    }

    #[test]
    fn operation_correlation_survives_resource_progress_and_completion() {
        let clock = Arc::new(ManualClock::default());
        let mut tracker = RequestTracker::new(2, clock);
        tracker
            .start_correlated(
                [8; 16],
                [7; 16],
                AddressHash::new([2; 16]),
                23,
                RequestOptions {
                    timeout: Duration::from_secs(5),
                    max_response_size: 64,
                    correlation_id: Some("page-correlation".into()),
                },
            )
            .expect("request starts");
        let hash = Hash::new([9; 32]);
        assert!(tracker.resource_advertised(AddressHash::new([2; 16]), [8; 16], hash, 3, 40));
        assert!(tracker.resource_progress(AddressHash::new([2; 16]), hash, 20, 40));
        assert!(tracker.resource_complete(
            AddressHash::new([2; 16]),
            hash,
            vec![1, 2, 3],
            None,
            40
        ));

        assert_eq!(
            tracker.get(&[8; 16]).and_then(|receipt| receipt.correlation_id.as_deref()),
            Some("page-correlation")
        );
    }
}
