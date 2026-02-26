/* Linker script for the QEMU `virt` RISC-V 32-bit machine.
 *
 * QEMU loads the ELF at the physical addresses and begins execution at
 * the ELF entry point (_start).
 *
 *   RAM: 0x8000_0000  (128 MiB reported by QEMU, we only use a fraction)
 */

ENTRY(_start)

MEMORY
{
    RAM (rwx) : ORIGIN = 0x80000000, LENGTH = 16M
}

SECTIONS
{
    .text : ALIGN(4)
    {
        *(.text._start)          /* entry point first */
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

    /* Kernel stack — 16 KiB, placed after BSS. */
    .stack (NOLOAD) : ALIGN(16)
    {
        __stack_bottom = .;
        . += 16K;
        __stack_top = .;
    } > RAM

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
    }
}
