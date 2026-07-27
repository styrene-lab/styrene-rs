//! Runtime diagnostic output policy.
//!
//! Library consumers such as the TUI must be able to embed the daemon without
//! process-global writes corrupting their renderer. The standalone daemon keeps
//! diagnostics enabled by default.

use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

#[macro_export]
macro_rules! daemon_diagnostic {
    ($($argument:tt)*) => {
        if $crate::diagnostics::enabled() {
            eprintln!($($argument)*);
        }
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn policy_can_be_disabled_and_restored() {
        super::set_enabled(false);
        assert!(!super::enabled());
        super::set_enabled(true);
        assert!(super::enabled());
    }
}
