/* Linker script for ESP32-C6 — Seeed XIAO ESP32-C6
 *
 * ESP32-C6 has 512 KB of HP SRAM at a unified address: instructions
 * and data share the same bus at 0x4080_0000.  The ROM bootloader
 * loads the application image from SPI flash into this SRAM region.
 *
 * Memory map (HP core):
 *   0x4000_0000 .. 0x4004_FFFF   320 KB  Boot ROM (read-only)
 *   0x4080_0000 .. 0x4087_FFFF   512 KB  HP SRAM (text + data + bss + stack)
 */

ENTRY(_start);

MEMORY
{
    RAM (rwx) : ORIGIN = 0x40800000, LENGTH = 512K
}

SECTIONS
{
    .text : ALIGN(4)
    {
        KEEP(*(.text._start));
        KEEP(*(.text._veer_trap_entry));
        KEEP(*(.text._veer_start_first_task));
        *(.text .text.*);
    } > RAM

    .rodata : ALIGN(4)
    {
        *(.rodata .rodata.*);
        *(.srodata .srodata.*);
    } > RAM

    .data : ALIGN(4)
    {
        *(.data .data.*);
        *(.sdata .sdata.*);
    } > RAM

    .bss (NOLOAD) : ALIGN(4)
    {
        __bss_start = .;
        *(.bss .bss.*);
        *(.sbss .sbss.*);
        *(COMMON);
        __bss_end = .;
    } > RAM

    /* Kernel boot stack — 8 KiB, placed after BSS. */
    .stack (NOLOAD) : ALIGN(16)
    {
        __stack_bottom = .;
        . += 8K;
        __stack_top = .;
    } > RAM

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
    }
}
