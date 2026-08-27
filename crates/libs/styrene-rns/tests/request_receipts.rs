#![cfg(feature = "transport")]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rns_core::hash::{AddressHash, Hash};
use rns_core::transport::request::{
    RequestClock, RequestStartError, RequestStatus, RequestTracker,
};

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

fn start(tracker: &mut RequestTracker, request_id: u8, link_id: u8, maximum: usize) {
    tracker
        .start(
            [request_id; 16],
            [7; 16],
            AddressHash::new([link_id; 16]),
            31,
            Duration::from_secs(5),
            maximum,
        )
        .expect("request receipt is created");
}

#[test]
fn terminal_receipts_reject_late_packet_and_resource_responses() {
    let clock = Arc::new(ManualClock::default());
    let mut tracker = RequestTracker::new(4, clock);
    start(&mut tracker, 1, 2, 8);
    assert!(tracker.cancel([1; 16]));
    assert!(!tracker.packet_response(AddressHash::new([2; 16]), [1; 16], b"late".to_vec(), 4));
    assert!(!tracker.resource_advertised(
        AddressHash::new([2; 16]),
        [1; 16],
        Hash::new([3; 32]),
        4,
        4
    ));
    assert_eq!(tracker.get(&[1; 16]).expect("retained").status, RequestStatus::Cancelled);
}

#[test]
fn monotonic_timeout_size_limit_progress_and_bounds_are_deterministic() {
    let clock = Arc::new(ManualClock::default());
    let mut tracker = RequestTracker::new(2, clock.clone());
    start(&mut tracker, 1, 2, 3);
    start(&mut tracker, 2, 2, 3);
    assert_eq!(
        tracker.start([3; 16], [7; 16], AddressHash::new([2; 16]), 1, Duration::from_secs(1), 1,),
        Err(RequestStartError::Capacity)
    );

    assert!(tracker.packet_response(AddressHash::new([2; 16]), [1; 16], vec![0; 4], 4));
    assert_eq!(tracker.get(&[1; 16]).expect("oversized").status, RequestStatus::ResponseTooLarge);
    start(&mut tracker, 3, 2, 3);
    assert!(!tracker.resource_advertised(
        AddressHash::new([2; 16]),
        [3; 16],
        Hash::new([8; 32]),
        4,
        4
    ));
    assert_eq!(
        tracker.get(&[3; 16]).expect("oversized resource").status,
        RequestStatus::ResponseTooLarge
    );
    let hash = Hash::new([9; 32]);
    assert!(tracker.resource_advertised(AddressHash::new([2; 16]), [2; 16], hash, 3, 3));
    assert!(tracker.resource_progress(AddressHash::new([2; 16]), hash, 2, 3));
    assert_eq!(tracker.get(&[2; 16]).expect("progress").progress, 2.0 / 3.0);

    clock.advance(Duration::from_secs(5));
    assert_eq!(tracker.poll_timeouts(), 1);
    assert_eq!(tracker.get(&[2; 16]).expect("timeout").status, RequestStatus::TimedOut);
}

#[test]
fn packet_and_resource_responses_from_wrong_link_are_rejected() {
    let clock = Arc::new(ManualClock::default());
    let mut tracker = RequestTracker::new(4, clock);
    let expected_link = AddressHash::new([2; 16]);
    let wrong_link = AddressHash::new([3; 16]);
    start(&mut tracker, 1, 2, 64);

    assert!(!tracker.packet_response(wrong_link, [1; 16], b"wrong".to_vec(), 5));
    assert_eq!(tracker.get(&[1; 16]).expect("pending packet").status, RequestStatus::Pending);

    let hash = Hash::new([9; 32]);
    assert!(!tracker.resource_advertised(wrong_link, [1; 16], hash, 32, 32));
    assert_eq!(tracker.get(&[1; 16]).expect("pending resource").status, RequestStatus::Pending);
    assert!(tracker.resource_advertised(expected_link, [1; 16], hash, 32, 32));
    assert!(!tracker.resource_progress(wrong_link, hash, 16, 32));
    assert!(!tracker.resource_complete(wrong_link, hash, b"wrong".to_vec(), 32));
    assert_eq!(tracker.get(&[1; 16]).expect("receiving").status, RequestStatus::Receiving);
}
