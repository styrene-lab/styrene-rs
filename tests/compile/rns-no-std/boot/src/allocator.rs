//! Finite-run allocator: one CPU, masked interrupts, no reclamation or reset.

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

// SAFETY: These symbols are defined by our linker script. Only their addresses
// are taken; no Rust reference to either sentinel byte is constructed or read.
unsafe extern "C" {
    static __heap_start: u8;
    static __heap_end: u8;
}

struct Bump {
    next: AtomicUsize,
}

#[global_allocator]
static HEAP: Bump = Bump { next: AtomicUsize::new(0) };

// SAFETY: Linker-owned heap is disjoint from image, tables and stack. Startup
// maps it as Normal RAM before Rust. Atomic reservations never overlap and the
// cursor never decreases. Deallocation does not invalidate any other pointer.
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let base = core::ptr::addr_of!(__heap_start) as usize;
        let end = core::ptr::addr_of!(__heap_end) as usize;
        let mut old = self.next.load(Ordering::Relaxed);
        loop {
            let cursor = if old == 0 { base } else { old };
            let Some((start, next)) =
                super::bump::reserve(cursor, end, layout.size(), layout.align())
            else {
                return core::ptr::null_mut();
            };
            match self.next.compare_exchange_weak(old, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return core::ptr::with_exposed_provenance_mut(start),
                Err(current) => old = current,
            }
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Finite workload only. Storage lives until QEMU exits. No reuse, reset,
        // dereference, allocation, formatting or unwinding is permitted here.
    }
}
