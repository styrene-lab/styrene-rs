//! Health monitoring for entropy sources.
//!
//! Implements the mandatory SP800-90B health tests:
//! - **Repetition Count Test (RCT)**: detects a stuck output value.
//! - **Adaptive Proportion Test (APT)**: detects severe bias in bit distribution.
//!
//! A source that fails either test transitions to [`SourceHealth::Degraded`]
//! and must not contribute to the pool until it is reset and re-probed.

use thiserror::Error;

/// Health state of an entropy source.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SourceHealth {
    /// Source is operating normally — contributing to pool.
    #[default]
    Ok,
    /// Source has failed a health test. Contains a human-readable reason.
    Degraded(String),
    /// Source is not reachable or not configured.
    Unavailable,
}

impl SourceHealth {
    /// Returns `true` if the source is healthy and should be polled.
    pub fn is_ok(&self) -> bool {
        matches!(self, SourceHealth::Ok)
    }
}

/// Error type for health test failures.
#[derive(Debug, Error, Clone)]
pub enum HealthError {
    /// Repetition Count Test failure — same byte repeated beyond threshold.
    #[error("RCT failure: byte 0x{byte:02x} repeated {count} times (limit {limit})")]
    RepetitionCount {
        /// The repeated byte value.
        byte: u8,
        /// How many times it appeared consecutively.
        count: usize,
        /// The configured limit.
        limit: usize,
    },
    /// Adaptive Proportion Test failure — bit distribution is too skewed.
    #[error("APT failure: {ones} ones in {window} bits ({pct:.1}% — expected ~50%)")]
    AdaptiveProportion {
        /// Number of 1-bits observed.
        ones: usize,
        /// Window size in bits.
        window: usize,
        /// Percentage of 1-bits.
        pct: f32,
    },
}

/// SP800-90B Repetition Count Test.
///
/// Fails if the same byte value appears more than `limit` times consecutively.
/// The limit is derived from the entropy estimate: for 1-bit-per-byte (conservative),
/// `limit = ceil(1 / H) = 1 / 0.5 = 2`; we use a practical default of 8.
#[derive(Debug, Default)]
pub struct RepetitionCountTest {
    last_byte: Option<u8>,
    run_length: usize,
    limit: usize,
}

impl RepetitionCountTest {
    /// Create a new RCT with the given consecutive-repeat limit.
    pub fn new(limit: usize) -> Self {
        Self { last_byte: None, run_length: 0, limit }
    }

    /// Feed one byte of source output. Returns `Err` on test failure.
    pub fn update(&mut self, byte: u8) -> Result<(), HealthError> {
        match self.last_byte {
            Some(last) if last == byte => {
                self.run_length += 1;
                if self.run_length >= self.limit {
                    return Err(HealthError::RepetitionCount {
                        byte,
                        count: self.run_length,
                        limit: self.limit,
                    });
                }
            }
            _ => {
                self.last_byte = Some(byte);
                self.run_length = 1;
            }
        }
        Ok(())
    }

    /// Reset the test state (e.g. after a source reset).
    pub fn reset(&mut self) {
        self.last_byte = None;
        self.run_length = 0;
    }
}

/// SP800-90B Adaptive Proportion Test.
///
/// Counts 1-bits in a window and fails if the proportion deviates too far from
/// the expected 50% (for a uniform source). Default window: 512 bits.
/// Failure thresholds: < 15% or > 85% ones.
#[derive(Debug)]
pub struct AdaptiveProportionTest {
    window_bits: usize,
    low_threshold: f32,
    high_threshold: f32,
    bit_buffer: Vec<u8>,
    bits_seen: usize,
}

impl Default for AdaptiveProportionTest {
    fn default() -> Self {
        Self::new(512, 0.15, 0.85)
    }
}

impl AdaptiveProportionTest {
    /// Create a new APT with the given window size and thresholds.
    pub fn new(window_bits: usize, low_threshold: f32, high_threshold: f32) -> Self {
        Self {
            window_bits,
            low_threshold,
            high_threshold,
            bit_buffer: Vec::new(),
            bits_seen: 0,
        }
    }

    /// Feed one byte of source output. Returns `Err` on test failure.
    pub fn update(&mut self, byte: u8) -> Result<(), HealthError> {
        self.bit_buffer.push(byte);
        self.bits_seen += 8;

        if self.bits_seen >= self.window_bits {
            let result = self.check_window();
            self.bit_buffer.clear();
            self.bits_seen = 0;
            return result;
        }
        Ok(())
    }

    fn check_window(&self) -> Result<(), HealthError> {
        let total_bits = self.bit_buffer.len() * 8;
        if total_bits == 0 {
            return Ok(());
        }
        let ones: usize = self.bit_buffer.iter().map(|b| b.count_ones() as usize).sum();
        let pct = ones as f32 / total_bits as f32;

        if pct < self.low_threshold || pct > self.high_threshold {
            return Err(HealthError::AdaptiveProportion {
                ones,
                window: total_bits,
                pct: pct * 100.0,
            });
        }
        Ok(())
    }

    /// Reset the test state.
    pub fn reset(&mut self) {
        self.bit_buffer.clear();
        self.bits_seen = 0;
    }
}

/// Combined health checker applying both SP800-90B mandatory tests.
#[derive(Debug)]
pub struct HealthChecker {
    rct: RepetitionCountTest,
    apt: AdaptiveProportionTest,
}

impl Default for HealthChecker {
    fn default() -> Self {
        Self { rct: RepetitionCountTest::new(8), apt: AdaptiveProportionTest::default() }
    }
}

impl HealthChecker {
    /// Feed a buffer of source output bytes through both tests.
    ///
    /// Returns the first failure encountered, or `Ok(())` if all bytes pass.
    pub fn update(&mut self, data: &[u8]) -> Result<(), HealthError> {
        for &byte in data {
            self.rct.update(byte)?;
            self.apt.update(byte)?;
        }
        Ok(())
    }

    /// Reset both tests (e.g. after a source hardware reset).
    pub fn reset(&mut self) {
        self.rct.reset();
        self.apt.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rct_passes_varied_data() {
        let mut rct = RepetitionCountTest::new(8);
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        for byte in data {
            rct.update(byte).expect("varied data should pass RCT");
        }
    }

    #[test]
    fn rct_fails_stuck_byte() {
        let mut rct = RepetitionCountTest::new(4);
        for _ in 0..3 {
            rct.update(0xAA).expect("first repeats should pass");
        }
        let result = rct.update(0xAA);
        assert!(result.is_err(), "should fail after limit repeats");
        assert!(matches!(result.unwrap_err(), HealthError::RepetitionCount { .. }));
    }

    #[test]
    fn apt_passes_uniform_data() {
        let mut apt = AdaptiveProportionTest::new(64, 0.15, 0.85);
        // 0x55 = 0101_0101 — exactly 50% ones
        for _ in 0..8 {
            apt.update(0x55).expect("balanced data should pass APT");
        }
    }

    #[test]
    fn apt_fails_all_zeros() {
        let mut apt = AdaptiveProportionTest::new(64, 0.15, 0.85);
        let result: Result<(), HealthError> =
            (0..8).try_for_each(|_| apt.update(0x00));
        assert!(result.is_err(), "all-zeros should fail APT");
    }

    #[test]
    fn apt_fails_all_ones() {
        let mut apt = AdaptiveProportionTest::new(64, 0.15, 0.85);
        let result: Result<(), HealthError> =
            (0..8).try_for_each(|_| apt.update(0xFF));
        assert!(result.is_err(), "all-ones should fail APT");
    }

    #[test]
    fn health_checker_combined() {
        let mut checker = HealthChecker::default();
        // Pseudo-random looking data — should pass both tests
        let data: Vec<u8> = (0u8..=127).collect();
        checker.update(&data).expect("varied data should pass combined health check");
    }

    #[test]
    fn rct_resets_cleanly() {
        let mut rct = RepetitionCountTest::new(4);
        for _ in 0..3 {
            rct.update(0xAA).expect("should pass");
        }
        rct.reset();
        // After reset, should be able to repeat again without failure
        for _ in 0..3 {
            rct.update(0xAA).expect("should pass after reset");
        }
    }
}
