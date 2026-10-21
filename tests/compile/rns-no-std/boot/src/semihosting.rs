//! Only SYS_WRITE0 and SYS_EXIT_EXTENDED. No host-file or console-input calls.

use core::arch::asm;

fn write(message: &'static [u8]) {
    // SAFETY: All callers below supply fixed NUL-terminated static byte strings.
    // Their identity-mapped storage remains readable for the synchronous QEMU
    // SYS_WRITE0 call. x0 is the operation/result, x1 the argument; other
    // registers are preserved by the AArch64 semihosting ABI. Memory is not
    // marked nomem/readonly, so the compiler cannot elide or reorder the read.
    unsafe {
        asm!("hlt #0xf000", inout("x0") 0x04usize => _, in("x1") message.as_ptr(),
            options(nostack));
    }
}

fn exit(code: u64) -> ! {
    let block = [0x20026u64, code]; // ADP_Stopped_ApplicationExit, subcode
    // SAFETY: The two-u64 ABI block is aligned and live on the mapped stack
    // during SYS_EXIT_EXTENDED. Only the fixed status codes below are used.
    unsafe {
        asm!("hlt #0xf000", inout("x0") 0x20usize => _, in("x1") block.as_ptr(),
            options(nostack));
    }
    // A runner that unexpectedly returns is never treated as successful.
    loop {
        core::hint::spin_loop();
    }
}

pub fn failure() -> ! {
    write(b"RNS-NOSTD:PANIC-OR-OOM\n\0");
    exit(1)
}

pub fn success() -> ! {
    #[cfg(feature = "smoke")]
    write(b"RNS-NOSTD:COMPLETE\n\0");
    #[cfg(not(feature = "smoke"))]
    write(b"RNS-NOSTD:BOOT-ONLY\n\0");
    exit(0)
}
