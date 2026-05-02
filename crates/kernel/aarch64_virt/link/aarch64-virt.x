/* Linker script for the generic AArch64 virtual-machine target. */

ENTRY(_start)

MEMORY
{
    RAM (rwx) : ORIGIN = 0x40080000, LENGTH = 512M
}

SECTIONS
{
    .text : ALIGN(4)
    {
        KEEP(*(.text._start))
        *(.text._veer_vectors)
        *(.text._veer_trap_sync)
        *(.text._veer_trap_irq)
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

    .stack (NOLOAD) : ALIGN(16)
    {
        __stack_bottom = .;
        . += 16M;
        __stack_top = .;
    } > RAM

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
    }
}
