/*
 * Linker script for VeerOS x86-64 kernel (QEMU q35/pc).
 *
 * Multiboot loads us at 1 MB physical (0x100000).
 * Identity-mapped at boot; higher-half deferred.
 */

ENTRY(_start)

MEMORY
{
    RAM (rwx) : ORIGIN = 0x100000, LENGTH = 16M
}

SECTIONS
{
    . = 0x100000;

    /* Multiboot header must appear in the first 8 KiB of the binary. */
    .multiboot : ALIGN(8)
    {
        KEEP(*(.multiboot))
    } > RAM

    .text : ALIGN(4)
    {
        *(.text._start)
        *(.text .text.*)
    } > RAM

    .rodata : ALIGN(4)
    {
        *(.rodata .rodata.*)
    } > RAM

    .data : ALIGN(4)
    {
        *(.data .data.*)
    } > RAM

    .bss (NOLOAD) : ALIGN(16)
    {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    } > RAM

    /* Bootstrap page tables — must be 4 KiB aligned.
       4 PDs cover the full 4 GiB identity map (for LAPIC/IOAPIC MMIO). */
    .page_tables (NOLOAD) : ALIGN(4096)
    {
        __pml4 = .;
        . += 4096;
        __pdpt = .;
        . += 4096;
        __pd = .;
        . += 4096 * 4;
    } > RAM

    .stack (NOLOAD) : ALIGN(16)
    {
        __stack_bottom = .;
        . += 32K;
        __stack_top = .;
    } > RAM

    /* End of kernel image — free physical memory starts here (page-aligned). */
    . = ALIGN(4096);
    __kernel_end = .;

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
        *(.note*)
    }
}
