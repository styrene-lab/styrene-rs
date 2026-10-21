//! QEMU virt-11.1, Cortex-A53, EL1, one CPU. See boot.S and linker.ld.

// SAFETY: The assembly establishes the linker-defined stack, zeroed BSS,
// exception vectors, FP/NEON and Normal-memory page tables before calling Rust.
// It has no caller and exports only the ELF entry and its vector handlers.
core::arch::global_asm!(include_str!("boot.S"));

// SAFETY: This unique symbol is called only by _start, after its documented
// machine-state initialization. The fixture contains no other definition.
#[unsafe(no_mangle)]
extern "C" fn fixture_entry() -> ! {
    super::run()
}
