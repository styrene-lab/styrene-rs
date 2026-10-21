ENTRY(_start)
MEMORY { RAM (rwx) : ORIGIN = 0x40080000, LENGTH = 0x07f80000 }
SECTIONS {
    .text : ALIGN(2048) { KEEP(*(.text.boot)) *(.text .text.*) } > RAM
    .rodata : ALIGN(16) { *(.rodata .rodata.*) } > RAM
    .data : ALIGN(16) { *(.data .data.*) *(.got .got.*) } > RAM
    .bss (NOLOAD) : ALIGN(16) {
        __bss_start = .;
        *(.bss .bss.*) *(COMMON)
        . = ALIGN(16);
        __bss_end = .;
    } > RAM
    .tables (NOLOAD) : ALIGN(4096) {
        __tables_start = .;
        . += 8192;
        __tables_end = .;
    } > RAM
    .heap (NOLOAD) : ALIGN(4096) {
        __heap_start = .;
        . += 16M;
        __heap_end = .;
    } > RAM
    .stack (NOLOAD) : ALIGN(4096) {
        __stack_bottom = .;
        . += 1M;
        __stack_top = .;
    } > RAM
    /DISCARD/ : { *(.eh_frame*) *(.comment) }
}
ASSERT(__bss_end <= __tables_start, "BSS overlaps tables")
ASSERT(__tables_end <= __heap_start, "tables overlap heap")
ASSERT(__heap_end <= __stack_bottom, "heap overlaps stack")
ASSERT((__tables_start & 4095) == 0, "unaligned tables")
ASSERT((__stack_top & 15) == 0, "unaligned stack")
ASSERT(__stack_top <= 0x48000000, "image exceeds mapped RAM")
