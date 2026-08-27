//! Runtime diagnostic output policy.
//!
//! Library consumers such as the TUI must be able to embed the daemon without
//! process-global writes corrupting their renderer. The standalone daemon keeps
//! diagnostics enabled by default.

use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::Mutex;

static ENABLED: AtomicBool = AtomicBool::new(true);
#[cfg(test)]
static CAPTURED: Mutex<Option<Vec<String>>> = Mutex::new(None);

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn should_emit() -> bool {
    if enabled() {
        return true;
    }
    #[cfg(test)]
    return CAPTURED.lock().is_ok_and(|captured| captured.is_some());
    #[cfg(not(test))]
    false
}

pub fn emit(arguments: std::fmt::Arguments<'_>) {
    #[cfg(test)]
    if let Ok(mut captured) = CAPTURED.lock() {
        if let Some(lines) = captured.as_mut() {
            lines.push(arguments.to_string());
            return;
        }
    }
    eprintln!("{arguments}");
}

#[cfg(test)]
pub fn start_capture() {
    *CAPTURED.lock().expect("diagnostic capture lock") = Some(Vec::new());
}

#[cfg(test)]
pub fn finish_capture() -> Vec<String> {
    CAPTURED.lock().expect("diagnostic capture lock").take().unwrap_or_default()
}

#[macro_export]
macro_rules! daemon_diagnostic {
    ($($argument:tt)*) => {
        if $crate::diagnostics::should_emit() {
            $crate::diagnostics::emit(format_args!($($argument)*));
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
