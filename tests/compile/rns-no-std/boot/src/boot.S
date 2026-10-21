// All addresses below are linker-owned, identity-mapped QEMU RAM. No MMIO.
.section .text.boot, "ax"
.global _start
.type _start, %function
_start:
    msr daifset, #0xf
    mrs x0, CurrentEL
    cmp x0, #4                 // require EL1, never guess an EL transition
    b.ne boot_failure
    mrs x0, sctlr_el1
    tbnz x0, #0, boot_failure  // this loader contract starts with MMU off
    tbnz x0, #2, boot_failure  // and no dirty D-cache to invalidate
    tbnz x0, #12, boot_failure // I-cache is initially disabled too
    mrs x0, mpidr_el1
    and x0, x0, #0xff
    cbnz x0, boot_failure
    msr spsel, #1
    ldr x0, =__stack_top
    mov sp, x0                // linker asserts 16-byte alignment
    ldr x0, =vectors
    msr vbar_el1, x0
    mov x0, #(3 << 20)
    msr cpacr_el1, x0          // enable FP/SIMD before any Rust code
    isb

    // Zero BSS, excluding our live stack, page tables and heap.
    ldr x0, =__bss_start
    ldr x1, =__bss_end
1:  cmp x0, x1
    b.hs 2f
    str xzr, [x0], #8
    b 1b
2:
    // Two 4 KiB tables: 39-bit VA starts at L1; entry 1 covers RAM.
    ldr x0, =__tables_start
    ldr x1, =__tables_end
3:  cmp x0, x1
    b.hs 4f
    str xzr, [x0], #8
    b 3b
4:  ldr x0, =__tables_start
    add x1, x0, #4096         // L2 table
    orr x2, x1, #3            // valid table descriptor
    str x2, [x0, #8]          // L1[1]: VA 0x40000000
    ldr x2, =0x0040000040000701 // UXN, AF, inner-shareable, AttrIdx0, 2MiB block
    mov x3, #64               // exactly 128 MiB; all other entries invalid
5:  str x2, [x1], #8
    add x2, x2, #0x200, lsl #12
    subs x3, x3, #1
    b.ne 5b
    dsb sy
    mov x1, #0xff             // MAIR[0]: Normal WB, read/write allocate
    msr mair_el1, x1
    // TCR: IPS=32-bit, TG1=4KiB (unused), EPD1=1, SH0=inner,
    // ORGN0/IRGN0=WBWA, TG0=4KiB, T0SZ=25 (39-bit VA).
    ldr x1, =((2 << 30) | (1 << 23) | (3 << 12) | (1 << 10) | (1 << 8) | 25)
    msr tcr_el1, x1
    msr ttbr0_el1, x0
    msr ttbr1_el1, xzr
    isb
    tlbi vmalle1
    dsb sy
    isb
    ldr x0, =0x30d0180d       // ARMv8 RES1, I/C/M enabled, SP alignment check
    msr sctlr_el1, x0
    isb
    bl fixture_entry
    b boot_failure
.size _start, . - _start

// Every EL1 vector slot terminates without entering Rust or allocating. A
// corrupted stack is not used. DAIF masks asynchronous exceptions beforehand.
.balign 2048
vectors:
.rept 16
    b boot_failure
    .balign 128
.endr

boot_failure:
    adr x1, trap_message
    mov x0, #4
    hlt #0xf000
    adr x1, trap_status
    mov x0, #0x20
    hlt #0xf000
6:  b 6b
.balign 8
trap_status:
    .quad 0x20026, 2
trap_message:
    .asciz "RNS-NOSTD:TRAP\n"
