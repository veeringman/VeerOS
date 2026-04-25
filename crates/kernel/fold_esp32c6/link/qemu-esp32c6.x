/* Linker script for the QEMU-emulated ESP32-C6 (RISC-V 32-bit).
 *
 * Uses the same QEMU virt machine as kernel-qemu-virt but with
 * constrained resources to emulate a real ESP32-C6:
 *   - 512 KiB SRAM (matching ESP32-C6 HP SRAM)
 *   - Entry at 0x8000_0000 (QEMU virt RAM base)
 */

ENTRY(_start)

MEMORY
{
    /* 16 MiB VM-backing RAM — the ESP32-C6 personality is enforced by the
     * kernel (16 KiB heap, constrained task stacks) rather than by the
     * linker.  QEMU needs enough room for its FDT + kernel image. */
    RAM (rwx) : ORIGIN = 0x80000000, LENGTH = 16M
}

SECTIONS
{
    .text : ALIGN(4)
    {
        *(.text._start)
        *(.text._veer_trap_entry)
        *(.text .text.*)
    } > RAM

    .rodata : ALIGN(4)
    {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    } > RAM

    .data : ALIGN(4)
    {
        *(.data .data.*)
        *(.sdata .sdata.*)
    } > RAM

    .bss (NOLOAD) : ALIGN(4)
    {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        __bss_end = .;
    } > RAM

    /* Kernel stack — 8 KiB (constrained for MCU emulation). */
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
