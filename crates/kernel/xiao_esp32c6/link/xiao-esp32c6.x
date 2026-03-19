/* Linker script for ESP32-C6 — Seeed XIAO ESP32-C6
 *
 * The ESP-IDF 2nd-stage bootloader expects the app image to contain:
 *   - Exactly 2 flash-mapped segments (DROM + IROM)
 *   - 1+ SRAM-loaded segments (data)
 *
 * Code (.text) and read-only data (.rodata) run from flash via the
 * instruction / data cache at 0x4200_0000.  Only mutable data (.data,
 * .bss, stacks, heap) is placed in the 512 KiB HP SRAM at 0x4080_0000.
 *
 * The 0x20 offset on FLASH accounts for the ESP image header (24 B)
 * plus the segment header (8 B) that espflash prepends to each segment.
 *
 * Memory map (HP core):
 *   0x4000_0000 .. 0x4004_FFFF   320 KB  Boot ROM (read-only)
 *   0x4080_0000 .. 0x4087_FFFF   512 KB  HP SRAM (data + bss + stack)
 *   0x4200_0000 .. 0x427F_FFFF    8 MB   Flash cache (IROM / DROM)
 */

ENTRY(_start);

MEMORY
{
    /* Flash-mapped data cache (read-only data, app descriptor).
       Placed first in the image at 0x4200_0020. */
    DROM  (r)  : ORIGIN = 0x42000020, LENGTH = 2M

    /* Flash-mapped instruction cache (code).
       Starts at a separate address (0x4220_0020) so espflash
       generates two distinct ROM segments for the bootloader. */
    IROM  (rx) : ORIGIN = 0x42200020, LENGTH = 2M

    /* On-chip SRAM for data, BSS, heap, and stacks. */
    DRAM  (rw) : ORIGIN = 0x40800000, LENGTH = 512K
}

SECTIONS
{
    /* ── DROM segment (read-only data, flash-mapped) ────────────
       Must come FIRST so it becomes segment 0 in the image —
       the bootloader checks segment 0 for the app descriptor magic. */
    .rodata : ALIGN(4)
    {
        _rodata_start = ABSOLUTE(.);
        KEEP(*(.veeros.appdesc));
        *(.rodata .rodata.*);
        *(.srodata .srodata.*);
        _rodata_end = ABSOLUTE(.);
    } > DROM

    /* ── IROM segment (code, flash-mapped) ──────────────────── */
    .text : ALIGN(4)
    {
        _stext = ABSOLUTE(.);
        KEEP(*(.text._start));
        KEEP(*(.text._veer_trap_entry));
        KEEP(*(.text._veer_start_first_task));
        *(.text .text.*);
        _etext = ABSOLUTE(.);
    } > IROM

    /* ── DRAM segment (mutable data, loaded to SRAM) ────────── */
    .data : ALIGN(4)
    {
        _data_start = ABSOLUTE(.);
        *(.data .data.*);
        *(.sdata .sdata.*);
        _data_end = ABSOLUTE(.);
    } > DRAM

    .bss (NOLOAD) : ALIGN(4)
    {
        __bss_start = .;
        *(.bss .bss.*);
        *(.sbss .sbss.*);
        *(COMMON);
        __bss_end = .;
    } > DRAM

    /* Kernel boot stack — 8 KiB, placed after BSS. */
    .stack (NOLOAD) : ALIGN(16)
    {
        __stack_bottom = .;
        . += 8K;
        __stack_top = .;
    } > DRAM

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
    }
}
