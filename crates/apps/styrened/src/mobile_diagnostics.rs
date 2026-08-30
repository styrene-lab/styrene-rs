use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac as _};
use rand_core::{OsRng, RngCore as _};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use styrene_ipc::types::{
    MOBILE_DIAGNOSTIC_MAX_BYTES, MOBILE_DIAGNOSTIC_MAX_EVENTS, MOBILE_DIAGNOSTIC_SCHEMA_VERSION,
    MobileDiagnosticEvent, MobileDiagnosticExport, MobileDiagnosticSeverity,
    MobileDiagnosticSnapshot, MobileDiagnosticSource, MobileDiagnosticStage,
};

// Leave room for the canonical envelope while enforcing the public 1 MiB artifact bound.
const MAX_RETAINED_EVENT_BYTES: usize = MOBILE_DIAGNOSTIC_MAX_BYTES as usize - 4096;
const BACKEND_REVISION: &str = concat!("styrened/", env!("CARGO_PKG_VERSION"));

struct RetainedEvent {
    sequence: u64,
    unix_time_ms: Option<u64>,
    source: MobileDiagnosticSource,
    stage: MobileDiagnosticStage,
    severity: MobileDiagnosticSeverity,
    generation: u64,
    safe_correlation: Option<String>,
    serialized_bytes: usize,
}

#[derive(Default)]
struct RingState {
    next_sequence: u64,
    retained_bytes: usize,
    dropped_events: u64,
    events: VecDeque<RetainedEvent>,
}

pub(crate) struct MobileDiagnostics {
    state: Mutex<RingState>,
    correlation_key: [u8; 32],
}

impl MobileDiagnostics {
    pub(crate) fn new() -> Result<Self, rand_core::Error> {
        let mut correlation_key = [0_u8; 32];
        OsRng.try_fill_bytes(&mut correlation_key)?;
        Ok(Self {
            state: Mutex::new(RingState { next_sequence: 1, ..RingState::default() }),
            correlation_key,
        })
    }

    pub(crate) fn record(
        &self,
        source: MobileDiagnosticSource,
        stage: MobileDiagnosticStage,
        severity: MobileDiagnosticSeverity,
        generation: u64,
        correlation: Option<&[u8]>,
    ) {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        let projected = MobileDiagnosticEvent {
            sequence,
            unix_time_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|duration| u64::try_from(duration.as_millis()).ok()),
            source,
            stage,
            severity,
            generation,
            safe_correlation: correlation.and_then(|value| self.safe_correlation(value)),
        };
        let serialized_bytes =
            serde_json::to_vec(&projected).map_or(usize::MAX, |bytes| bytes.len());
        let retained = RetainedEvent {
            sequence: projected.sequence,
            unix_time_ms: projected.unix_time_ms,
            source: projected.source,
            stage: projected.stage,
            severity: projected.severity,
            generation: projected.generation,
            safe_correlation: projected.safe_correlation,
            serialized_bytes,
        };
        state.retained_bytes = state.retained_bytes.saturating_add(serialized_bytes);
        state.events.push_back(retained);
        while state.events.len() > MOBILE_DIAGNOSTIC_MAX_EVENTS as usize
            || state.retained_bytes > MAX_RETAINED_EVENT_BYTES
        {
            if let Some(removed) = state.events.pop_front() {
                state.retained_bytes =
                    state.retained_bytes.saturating_sub(removed.serialized_bytes);
                state.dropped_events = state.dropped_events.saturating_add(1);
            }
        }
    }

    fn safe_correlation(&self, value: &[u8]) -> Option<String> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.correlation_key).ok()?;
        mac.update(value);
        Some(format!("hmac-sha256:{}", hex::encode(mac.finalize().into_bytes())))
    }

    pub(crate) fn snapshot(&self) -> MobileDiagnosticSnapshot {
        let state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let events = state.events.iter().map(redact_event).collect::<Vec<_>>();
        MobileDiagnosticSnapshot {
            schema_version: MOBILE_DIAGNOSTIC_SCHEMA_VERSION,
            backend_revision: BACKEND_REVISION.to_string(),
            first_sequence: events.first().map(|event| event.sequence),
            last_sequence: events.last().map(|event| event.sequence),
            event_count: events.len() as u32,
            retained_bytes: state.retained_bytes as u64,
            max_events: MOBILE_DIAGNOSTIC_MAX_EVENTS,
            max_bytes: MOBILE_DIAGNOSTIC_MAX_BYTES,
            truncated: state.dropped_events != 0,
            dropped_events: state.dropped_events,
            events,
        }
    }

    pub(crate) fn export(&self) -> Result<MobileDiagnosticExport, serde_json::Error> {
        let snapshot = self.snapshot();
        let bytes = serde_json::to_vec(&CanonicalExport::from(&snapshot))?;
        debug_assert!(bytes.len() <= MOBILE_DIAGNOSTIC_MAX_BYTES as usize);
        Ok(MobileDiagnosticExport {
            schema_version: snapshot.schema_version,
            backend_revision: snapshot.backend_revision,
            content_type: "application/vnd.styrene.mobile-diagnostics+json".to_string(),
            digest_sha256: hex::encode(Sha256::digest(&bytes)),
            first_sequence: snapshot.first_sequence,
            last_sequence: snapshot.last_sequence,
            event_count: snapshot.event_count,
            byte_count: bytes.len() as u64,
            max_events: snapshot.max_events,
            max_bytes: snapshot.max_bytes,
            truncated: snapshot.truncated,
            dropped_events: snapshot.dropped_events,
            bytes,
        })
    }
}

fn redact_event(event: &RetainedEvent) -> MobileDiagnosticEvent {
    MobileDiagnosticEvent {
        sequence: event.sequence,
        unix_time_ms: event.unix_time_ms,
        source: event.source,
        stage: event.stage,
        severity: event.severity,
        generation: event.generation,
        safe_correlation: event.safe_correlation.clone(),
    }
}

#[derive(Serialize)]
struct CanonicalExport<'a> {
    schema_version: u32,
    backend_revision: &'a str,
    first_sequence: Option<u64>,
    last_sequence: Option<u64>,
    event_count: u32,
    retained_bytes: u64,
    max_events: u32,
    max_bytes: u64,
    truncated: bool,
    dropped_events: u64,
    events: &'a [MobileDiagnosticEvent],
}

impl<'a> From<&'a MobileDiagnosticSnapshot> for CanonicalExport<'a> {
    fn from(snapshot: &'a MobileDiagnosticSnapshot) -> Self {
        Self {
            schema_version: snapshot.schema_version,
            backend_revision: &snapshot.backend_revision,
            first_sequence: snapshot.first_sequence,
            last_sequence: snapshot.last_sequence,
            event_count: snapshot.event_count,
            retained_bytes: snapshot.retained_bytes,
            max_events: snapshot.max_events,
            max_bytes: snapshot.max_bytes,
            truncated: snapshot.truncated,
            dropped_events: snapshot.dropped_events,
            events: &snapshot.events,
        }
    }
}
