//! Protocol wall-clock time for timestamp-dependent operations.
//!
//! Announce random blobs carry a Unix timestamp and ratchet rotation is
//! measured in Unix seconds, so both need one wall-clock source. With the
//! `std` feature the system clock is that source. Without it the embedding
//! application supplies whole-second Unix time and refreshes it as time
//! advances; until it does, timestamp-dependent operations return
//! [`RnsError::TimeUnavailable`] instead of panicking or emitting an epoch
//! timestamp.

use crate::error::RnsError;

/// Current Unix time in whole seconds.
#[cfg(feature = "std")]
pub fn unix_now() -> Result<u64, RnsError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|_| RnsError::TimeUnavailable)
}

#[cfg(not(feature = "std"))]
mod embedded {
    use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    static INITIALIZED: AtomicBool = AtomicBool::new(false);
    static UNIX_SECS: AtomicU64 = AtomicU64::new(0);

    pub fn get() -> Option<u64> {
        INITIALIZED.load(Ordering::Acquire).then(|| UNIX_SECS.load(Ordering::Acquire))
    }

    pub fn set(secs: u64) {
        UNIX_SECS.store(secs, Ordering::Release);
        INITIALIZED.store(true, Ordering::Release);
    }

    pub fn advance(delta_secs: u64) -> Option<u64> {
        if !INITIALIZED.load(Ordering::Acquire) {
            return None;
        }
        let mut current = UNIX_SECS.load(Ordering::Acquire);
        loop {
            let next = current.saturating_add(delta_secs);
            match UNIX_SECS.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(next),
                Err(observed) => current = observed,
            }
        }
    }

    pub fn clear() {
        INITIALIZED.store(false, Ordering::Release);
        UNIX_SECS.store(0, Ordering::Release);
    }
}

/// Current Unix time in whole seconds as supplied by the embedding.
#[cfg(not(feature = "std"))]
pub fn unix_now() -> Result<u64, RnsError> {
    embedded::get().ok_or(RnsError::TimeUnavailable)
}

/// Supply the current Unix time in whole seconds. Calling this again with a
/// newer value refreshes the clock; it never moves time backwards on its own.
#[cfg(not(feature = "std"))]
pub fn set_unix_time(secs: u64) {
    embedded::set(secs);
}

/// Advance the supplied Unix time by `delta_secs`. Returns the new time, or
/// `None` when no time has been supplied yet.
#[cfg(not(feature = "std"))]
pub fn advance_unix_time(delta_secs: u64) -> Option<u64> {
    embedded::advance(delta_secs)
}

/// Forget the supplied time so timestamp-dependent operations fail again.
/// Intended for embeddings that lose their clock and for tests.
#[cfg(not(feature = "std"))]
pub fn clear_unix_time() {
    embedded::clear();
}

/// Current Unix time as fractional seconds, for ratchet age arithmetic.
pub fn unix_now_secs_f64() -> Result<f64, RnsError> {
    unix_now().map(|secs| secs as f64)
}
