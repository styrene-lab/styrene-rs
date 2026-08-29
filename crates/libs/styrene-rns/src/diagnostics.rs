use core::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn emit(arguments: core::fmt::Arguments<'_>) {
    use std::io::Write as _;

    // Diagnostic output must not abort transport work when an embedded host
    // does not provide a writable stderr descriptor.
    let _ = writeln!(std::io::stderr().lock(), "{arguments}");
}

#[macro_export]
macro_rules! transport_diagnostic {
    ($($argument:tt)*) => {
        if $crate::diagnostics::enabled() {
            $crate::diagnostics::emit(format_args!($($argument)*));
        }
    };
}
