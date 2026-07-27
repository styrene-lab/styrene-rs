use core::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

#[macro_export]
macro_rules! transport_diagnostic {
    ($($argument:tt)*) => {
        if $crate::diagnostics::enabled() {
            eprintln!($($argument)*);
        }
    };
}
