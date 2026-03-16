/* Linker script for Raspberry Pi 5 (AArch64).
 *
 * The RPi firmware loads kernel8.img as a flat binary at 0x80000.
 * We use that as our entry point and place everything in RAM.
 * Conservative 256 MB region — the kernel never uses more than a
 * few hundred KiB.
 */

ENTRY(_start)

MEMORY
{
    RAM (rwx) : ORIGIN = 0x80000, LENGTH = 256M
}

SECTIONS
{
    .text : ALIGN(4)
    {
        KEEP(*(.text._start))
        *(.text._veer_vectors)
        *(.text._veer_trap_sync)
        *(.text._veer_trap_irq)
        *(.text._veer_start_first_task)
        *(.text .text.*)
    } > RAM

    .rodata : ALIGN(8)
    {
        *(.rodata .rodata.*)
    } > RAM

    .data : ALIGN(8)
    {
        *(.data .data.*)
    } > RAM

    .bss (NOLOAD) : ALIGN(8)
    {
        __bss_start = .;
        *(.bss .bss.*)
        __bss_end = .;
    } > RAM

    /* Kernel stack — 32 KiB (bigger than RISC-V due to 8-byte regs). */
    .stack (NOLOAD) : ALIGN(16)
    {
        __stack_bottom = .;
        . += 32K;
        __stack_top = .;
    } > RAM

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
    }
}
