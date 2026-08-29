use std::array;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use crate::RnsError;
use crate::hash::AddressHash;
use crate::packet::PacketType;

use super::RxMessage;

const CLASS_COUNT: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressClass {
    Data,
    Announce,
    PathRequest,
    IngressLimited,
}

impl IngressClass {
    const fn index(self) -> usize {
        match self {
            Self::Data => 0,
            Self::Announce => 1,
            Self::PathRequest => 2,
            Self::IngressLimited => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngressQueueCapacities {
    values: [usize; CLASS_COUNT],
}

impl IngressQueueCapacities {
    pub const DEFAULT: Self = Self { values: [1024, 128, 128, 8] };

    pub fn new(
        data: usize,
        announce: usize,
        path_request: usize,
        ingress_limited: usize,
    ) -> Result<Self, RnsError> {
        let values = [data, announce, path_request, ingress_limited];
        if values.contains(&0) {
            return Err(RnsError::InvalidArgument);
        }
        Ok(Self { values })
    }

    pub(crate) fn uniform(capacity: usize) -> Self {
        assert!(capacity > 0, "ingress queue capacity must be positive");
        Self { values: [capacity; CLASS_COUNT] }
    }

    pub const fn data(self) -> usize {
        self.values[0]
    }

    pub const fn announce(self) -> usize {
        self.values[1]
    }

    pub const fn path_request(self) -> usize {
        self.values[2]
    }

    pub const fn ingress_limited(self) -> usize {
        self.values[3]
    }
}

impl Default for IngressQueueCapacities {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IngressClassSnapshot {
    pub capacity: u64,
    pub depth: u64,
    pub dropped: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IngressSnapshot {
    pub data: IngressClassSnapshot,
    pub announce: IngressClassSnapshot,
    pub path_request: IngressClassSnapshot,
    pub ingress_limited: IngressClassSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressEnqueueOutcome {
    Accepted,
    Dropped,
    Rejected,
}

#[derive(Debug)]
pub struct InterfaceRxSendError;

struct IngressState {
    queues: [VecDeque<RxMessage>; CLASS_COUNT],
    capacities: [usize; CLASS_COUNT],
    dropped: [u64; CLASS_COUNT],
    sender_count: usize,
    receiver_open: bool,
}

struct IngressShared {
    state: Mutex<IngressState>,
    admission: Mutex<Option<Arc<IngressAdmission>>>,
    notify: Notify,
    path_request_destination: AddressHash,
}

type IngressAdmissionFuture =
    Pin<Box<dyn Future<Output = Result<Option<RxMessage>, InterfaceRxSendError>> + Send>>;
type IngressAdmission = dyn Fn(RxMessage) -> IngressAdmissionFuture + Send + Sync;

pub struct InterfaceRxSender {
    shared: Arc<IngressShared>,
}

pub struct InterfaceRxReceiver {
    shared: Arc<IngressShared>,
}

impl Clone for InterfaceRxSender {
    fn clone(&self) -> Self {
        self.shared.state.lock().expect("ingress queue lock").sender_count += 1;
        Self { shared: self.shared.clone() }
    }
}

impl Drop for InterfaceRxSender {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().expect("ingress queue lock");
        state.sender_count = state.sender_count.saturating_sub(1);
        let closed = state.sender_count == 0;
        drop(state);
        if closed {
            self.shared.notify.notify_one();
        }
    }
}

impl Drop for InterfaceRxReceiver {
    fn drop(&mut self) {
        self.shared.state.lock().expect("ingress queue lock").receiver_open = false;
        self.shared.notify.notify_waiters();
    }
}

impl InterfaceRxSender {
    pub(crate) fn channel(
        capacities: IngressQueueCapacities,
        path_request_destination: AddressHash,
    ) -> (Self, InterfaceRxReceiver) {
        let shared = Arc::new(IngressShared {
            state: Mutex::new(IngressState {
                queues: array::from_fn(|_| VecDeque::new()),
                capacities: capacities.values,
                dropped: [0; CLASS_COUNT],
                sender_count: 1,
                receiver_open: true,
            }),
            admission: Mutex::new(None),
            notify: Notify::new(),
            path_request_destination,
        });
        (Self { shared: shared.clone() }, InterfaceRxReceiver { shared })
    }

    fn classify(&self, message: &RxMessage) -> IngressClass {
        if message.ingress_class == IngressClass::IngressLimited {
            IngressClass::IngressLimited
        } else if message.packet.header.packet_type == PacketType::Announce {
            IngressClass::Announce
        } else if message.packet.header.packet_type == PacketType::Data
            && message.packet.destination == self.shared.path_request_destination
        {
            IngressClass::PathRequest
        } else {
            IngressClass::Data
        }
    }

    fn enqueue(
        &self,
        mut message: RxMessage,
    ) -> Result<IngressEnqueueOutcome, InterfaceRxSendError> {
        let class = self.classify(&message);
        message.ingress_class = class;
        let index = class.index();
        let mut state = self.shared.state.lock().expect("ingress queue lock");
        if !state.receiver_open {
            return Err(InterfaceRxSendError);
        }
        if state.queues[index].len() >= state.capacities[index] {
            state.dropped[index] = state.dropped[index].saturating_add(1);
            return Ok(IngressEnqueueOutcome::Dropped);
        }
        state.queues[index].push_back(message);
        drop(state);
        self.shared.notify.notify_one();
        Ok(IngressEnqueueOutcome::Accepted)
    }

    pub(crate) fn set_admission<F>(&self, admission: F)
    where
        F: Fn(RxMessage) -> IngressAdmissionFuture + Send + Sync + 'static,
    {
        *self.shared.admission.lock().expect("ingress admission lock") = Some(Arc::new(admission));
    }

    pub async fn send(
        &self,
        message: RxMessage,
    ) -> Result<IngressEnqueueOutcome, InterfaceRxSendError> {
        let admission = self.shared.admission.lock().expect("ingress admission lock").clone();
        let message = if let Some(admission) = admission {
            let Some(message) = admission(message).await? else {
                return Ok(IngressEnqueueOutcome::Rejected);
            };
            message
        } else {
            message
        };
        self.enqueue(message)
    }

    pub fn snapshot(&self) -> IngressSnapshot {
        snapshot(&self.shared.state.lock().expect("ingress queue lock"))
    }
}

impl InterfaceRxReceiver {
    pub async fn recv(&mut self) -> Option<RxMessage> {
        loop {
            let notified = self.shared.notify.notified();
            {
                let mut state = self.shared.state.lock().expect("ingress queue lock");
                for queue in &mut state.queues {
                    if let Some(message) = queue.pop_front() {
                        return Some(message);
                    }
                }
                if state.sender_count == 0 {
                    return None;
                }
            }
            notified.await;
        }
    }

    pub fn snapshot(&self) -> IngressSnapshot {
        snapshot(&self.shared.state.lock().expect("ingress queue lock"))
    }
}

fn class_snapshot(state: &IngressState, class: IngressClass) -> IngressClassSnapshot {
    let index = class.index();
    IngressClassSnapshot {
        capacity: state.capacities[index] as u64,
        depth: state.queues[index].len() as u64,
        dropped: state.dropped[index],
    }
}

fn snapshot(state: &IngressState) -> IngressSnapshot {
    IngressSnapshot {
        data: class_snapshot(state, IngressClass::Data),
        announce: class_snapshot(state, IngressClass::Announce),
        path_request: class_snapshot(state, IngressClass::PathRequest),
        ingress_limited: class_snapshot(state, IngressClass::IngressLimited),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{Header, Packet, PacketDataBuffer};

    fn message(id: u8, packet_type: PacketType, destination: AddressHash) -> RxMessage {
        RxMessage::local(
            AddressHash::new([0x11; 16]),
            Packet {
                header: Header { packet_type, ..Default::default() },
                destination,
                data: PacketDataBuffer::new_from_slice(&[id]),
                ..Default::default()
            },
        )
    }

    fn id(message: RxMessage) -> u8 {
        message.packet.data.as_slice()[0]
    }

    #[test]
    fn defaults_and_positive_override_validation_are_canonical() {
        assert_eq!(IngressQueueCapacities::default().values, [1024, 128, 128, 8]);
        assert!(IngressQueueCapacities::new(1, 2, 3, 4).is_ok());
        assert!(IngressQueueCapacities::new(0, 2, 3, 4).is_err());
    }

    #[tokio::test]
    async fn strict_priority_restarts_at_data_and_preserves_fifo() {
        let path = AddressHash::new([0x22; 16]);
        let other = AddressHash::new([0x33; 16]);
        let capacities = IngressQueueCapacities::new(4, 4, 4, 4).unwrap();
        let (sender, mut receiver) = InterfaceRxSender::channel(capacities, path);

        sender.enqueue(message(40, PacketType::Announce, other).ingress_limited()).unwrap();
        sender.enqueue(message(41, PacketType::Data, path).ingress_limited()).unwrap();
        sender.enqueue(message(30, PacketType::Data, path)).unwrap();
        sender.enqueue(message(31, PacketType::Data, path)).unwrap();
        sender.enqueue(message(20, PacketType::Announce, other)).unwrap();
        sender.enqueue(message(21, PacketType::Announce, other)).unwrap();
        sender.enqueue(message(10, PacketType::Data, other)).unwrap();
        sender.enqueue(message(11, PacketType::Proof, other)).unwrap();
        sender.enqueue(message(13, PacketType::Proof, path)).unwrap();

        assert_eq!(id(receiver.recv().await.unwrap()), 10);
        sender.enqueue(message(12, PacketType::LinkRequest, other)).unwrap();
        assert_eq!(id(receiver.recv().await.unwrap()), 11);
        assert_eq!(id(receiver.recv().await.unwrap()), 13);
        assert_eq!(id(receiver.recv().await.unwrap()), 12);
        assert_eq!(id(receiver.recv().await.unwrap()), 20);
        assert_eq!(id(receiver.recv().await.unwrap()), 21);
        assert_eq!(id(receiver.recv().await.unwrap()), 30);
        assert_eq!(id(receiver.recv().await.unwrap()), 31);
        assert_eq!(id(receiver.recv().await.unwrap()), 40);
        assert_eq!(id(receiver.recv().await.unwrap()), 41);
    }

    #[tokio::test]
    async fn sustained_data_starves_lower_classes_until_data_empties() {
        let path = AddressHash::new([0x22; 16]);
        let other = AddressHash::new([0x33; 16]);
        let capacities = IngressQueueCapacities::new(2, 2, 2, 2).unwrap();
        let (sender, mut receiver) = InterfaceRxSender::channel(capacities, path);
        sender.enqueue(message(250, PacketType::Announce, other)).unwrap();
        sender.enqueue(message(251, PacketType::Announce, other)).unwrap();
        sender.enqueue(message(1, PacketType::Data, other)).unwrap();

        for next in 2..=100 {
            assert_ne!(id(receiver.recv().await.unwrap()), 250);
            sender.enqueue(message(next, PacketType::Data, other)).unwrap();
        }
        assert_ne!(id(receiver.recv().await.unwrap()), 250);
        assert_eq!(id(receiver.recv().await.unwrap()), 250);
        assert_eq!(id(receiver.recv().await.unwrap()), 251);
    }

    #[tokio::test]
    async fn overflow_drops_new_item_and_snapshot_changes_one_class_only() {
        let path = AddressHash::new([0x22; 16]);
        let other = AddressHash::new([0x33; 16]);
        let capacities = IngressQueueCapacities::new(2, 2, 2, 1).unwrap();
        let (sender, mut receiver) = InterfaceRxSender::channel(capacities, path);
        assert_eq!(
            sender.enqueue(message(1, PacketType::Data, other)).unwrap(),
            IngressEnqueueOutcome::Accepted
        );
        sender.enqueue(message(2, PacketType::Data, other)).unwrap();
        assert_eq!(
            sender.send(message(3, PacketType::Data, other)).await.unwrap(),
            IngressEnqueueOutcome::Dropped
        );

        assert_eq!(
            sender.snapshot(),
            IngressSnapshot {
                data: IngressClassSnapshot { capacity: 2, depth: 2, dropped: 1 },
                announce: IngressClassSnapshot { capacity: 2, depth: 0, dropped: 0 },
                path_request: IngressClassSnapshot { capacity: 2, depth: 0, dropped: 0 },
                ingress_limited: IngressClassSnapshot { capacity: 1, depth: 0, dropped: 0 },
            }
        );
        assert_eq!(id(receiver.recv().await.unwrap()), 1);
        assert_eq!(id(receiver.recv().await.unwrap()), 2);
    }

    #[test]
    fn concurrent_producers_cannot_exceed_class_capacities() {
        let path = AddressHash::new([0x22; 16]);
        let other = AddressHash::new([0x33; 16]);
        let capacities = IngressQueueCapacities::new(16, 8, 4, 2).unwrap();
        let (sender, _receiver) = InterfaceRxSender::channel(capacities, path);
        let threads: Vec<_> = (0..8)
            .map(|thread| {
                let sender = sender.clone();
                std::thread::spawn(move || {
                    for item in 0..16 {
                        let id = thread * 16 + item;
                        let _ = sender.enqueue(message(id, PacketType::Data, other));
                        let _ = sender.enqueue(message(id, PacketType::Announce, other));
                        let _ = sender.enqueue(message(id, PacketType::Data, path));
                        let _ =
                            sender.enqueue(message(id, PacketType::Data, other).ingress_limited());
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let snapshot = sender.snapshot();
        assert_eq!(snapshot.data.depth, 16);
        assert_eq!(snapshot.data.dropped, 112);
        assert_eq!(snapshot.announce.depth, 8);
        assert_eq!(snapshot.announce.dropped, 120);
        assert_eq!(snapshot.path_request.depth, 4);
        assert_eq!(snapshot.path_request.dropped, 124);
        assert_eq!(snapshot.ingress_limited.depth, 2);
        assert_eq!(snapshot.ingress_limited.dropped, 126);
    }
}
