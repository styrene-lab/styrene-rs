use std::time::Duration;
use tokio::time::Instant;

const MTU_BITS: u64 = crate::packet::MTU as u64 * 8;
const MINIMUM_BITRATE: u64 = 5;
const DEFAULT_PER_HOP_GRACE: Duration = Duration::from_secs(6);

fn python_seconds(seconds: f64) -> Duration {
    Duration::from_nanos((seconds * 1_000_000_000.0).round_ties_even() as u64)
}

pub(crate) fn deadline(now: Instant, timeout: Duration) -> Instant {
    if let Some(deadline) = now.checked_add(timeout) {
        return deadline;
    }

    let mut low = 0_u128;
    let mut high = timeout.as_nanos();
    while low < high {
        let midpoint = low + (high - low).div_ceil(2);
        let duration =
            Duration::new((midpoint / 1_000_000_000) as u64, (midpoint % 1_000_000_000) as u32);
        if now.checked_add(duration).is_some() {
            low = midpoint;
        } else {
            high = midpoint - 1;
        }
    }
    let duration = Duration::new((low / 1_000_000_000) as u64, (low % 1_000_000_000) as u32);
    now.checked_add(duration).unwrap_or(now)
}

pub fn medium_path_grace(lowest_online_bitrate: Option<u64>) -> Duration {
    let Some(bitrate) = lowest_online_bitrate.filter(|bitrate| *bitrate > 0) else {
        return Duration::ZERO;
    };
    let seconds = 2.0 * (MTU_BITS as f64 / bitrate.max(MINIMUM_BITRATE) as f64)
        + DEFAULT_PER_HOP_GRACE.as_secs_f64();
    python_seconds(seconds)
}

pub fn discovery_timeout(
    configured_timeout: Duration,
    lowest_online_bitrate: Option<u64>,
) -> Duration {
    configured_timeout.max(medium_path_grace(lowest_online_bitrate))
}

pub fn link_proof_extra_grace(outbound_bitrate: Option<u64>) -> Duration {
    outbound_bitrate
        .filter(|bitrate| *bitrate > 0)
        .map(|bitrate| python_seconds(MTU_BITS as f64 / bitrate as f64))
        .unwrap_or_default()
}

pub fn link_proof_timeout(
    fixed_timeout: Option<Duration>,
    per_hop_timeout: Duration,
    remaining_hops: u8,
    outbound_bitrate: Option<u64>,
) -> Duration {
    if let Some(fixed_timeout) = fixed_timeout {
        return fixed_timeout;
    }

    per_hop_timeout
        .checked_mul(u32::from(remaining_hops.max(1)))
        .and_then(|base| base.checked_add(link_proof_extra_grace(outbound_bitrate)))
        .unwrap_or(Duration::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_discovery_fallback_and_proof_base_are_finite() {
        assert_eq!(discovery_timeout(Duration::from_secs(30), None), Duration::from_secs(30));
        assert_eq!(discovery_timeout(Duration::from_secs(30), Some(100)), Duration::from_secs(86));
        assert_eq!(
            link_proof_timeout(None, Duration::from_secs(6), 0, None),
            Duration::from_secs(6)
        );
        assert_eq!(
            link_proof_timeout(None, Duration::from_secs(6), 3, Some(500)),
            Duration::from_secs(26)
        );
        assert_eq!(
            link_proof_timeout(
                Some(Duration::from_secs(600)),
                Duration::from_secs(6),
                3,
                Some(500)
            ),
            Duration::from_secs(600)
        );
    }

    #[test]
    fn deadline_arithmetic_saturates_without_panicking_or_arbitrary_cap() {
        assert_eq!(link_proof_timeout(None, Duration::MAX, u8::MAX, Some(1)), Duration::MAX);
        let now = Instant::now();
        assert!(deadline(now, Duration::MAX) >= now);
        let two_years = Duration::from_secs(2 * 365 * 24 * 60 * 60);
        assert_eq!(deadline(now, two_years), now + two_years);
    }

    #[test]
    fn transmission_rounding_matches_python_nearest_nanosecond() {
        assert_eq!(link_proof_extra_grace(Some(6)), Duration::from_nanos(666_666_666_667));
        assert_eq!(medium_path_grace(Some(9)), Duration::from_nanos(894_888_888_889));
        assert_eq!(medium_path_grace(Some(2_243_903)), Duration::from_nanos(6_003_565_216));
    }
}
