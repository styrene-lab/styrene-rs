//! EventService — event bus, notifications, activity ring.
//!
//! Owns: 5.1 EventBus, 5.2 notifications, 5.3 activity ring, event fan-out to IPC/SSE.
//! Package: H

#[derive(Default)]
pub struct EventService {
    // Fields will be added in Package H
}

impl EventService {
    pub fn new() -> Self {
        Self {}
    }
}
