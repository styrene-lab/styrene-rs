use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub trait MonotonicClock: Send + Sync {
    fn now(&self) -> Duration;
}

#[derive(Debug, Default)]
pub struct SystemMonotonicClock;

impl MonotonicClock for SystemMonotonicClock {
    fn now(&self) -> Duration {
        monotonic_now()
    }
}

pub(crate) fn monotonic_now() -> Duration {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed()
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct ManualMonotonicClock(std::sync::atomic::AtomicU64);

#[cfg(test)]
impl ManualMonotonicClock {
    pub(crate) fn advance(&self, duration: Duration) {
        self.0.fetch_add(
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            std::sync::atomic::Ordering::SeqCst,
        );
    }
}

#[cfg(test)]
impl MonotonicClock for ManualMonotonicClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }
}

pub fn now_epoch_secs_u64() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

pub fn now_epoch_secs_i64() -> i64 {
    i64::try_from(now_epoch_secs_u64()).unwrap_or(0)
}
