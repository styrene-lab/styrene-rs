#![no_std]
#![no_main]
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

extern crate alloc;

// The only fixture-specific unsafe exceptions. Product lints are not inherited
// or overridden. Each operation's safety contract resides in its owning module.
#[allow(unsafe_code)]
mod allocator;
mod bump;
#[allow(unsafe_code)]
mod semihosting;
#[allow(unsafe_code)]
mod startup;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    // No formatting, allocation or attempt to unwind. This also handles the
    // pinned no_std alloc runtime's default allocation-error panic.
    semihosting::failure()
}

fn run() -> ! {
    // Exercise the boot substrate even without a product dependency: heap,
    // atomics on Normal RAM, floating point, and non-overlap of live objects.
    let value = alloc::sync::Arc::new(core::sync::atomic::AtomicU64::new(7));
    let alias = value.clone();
    assert_eq!(alias.fetch_add(1, core::sync::atomic::Ordering::SeqCst), 7);
    assert_eq!(value.load(core::sync::atomic::Ordering::SeqCst), 8);
    let float = core::hint::black_box(3u64) as f64;
    assert_eq!(core::hint::black_box(float) * 2.0, 6.0);

    if cfg!(feature = "scenario-panic") {
        panic!("intentional synthetic panic");
    }
    if cfg!(feature = "scenario-oom") {
        let mut too_large = alloc::vec::Vec::<u8>::with_capacity(core::hint::black_box(17 << 20));
        too_large.push(1);
        core::hint::black_box(too_large);
        // Reaching this means the OOM control is broken; never report success.
        semihosting::failure();
    }
    #[cfg(feature = "smoke")]
    rns_no_std_smoke::run();
    semihosting::success()
}
