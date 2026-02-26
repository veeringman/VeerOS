ENTRY(_start);

MEMORY
{
  ROM (rx) : ORIGIN = 0x40380000, LENGTH = 384K
  RAM (rwx) : ORIGIN = 0x3FC80000, LENGTH = 400K
}

SECTIONS
{
  .text :
  {
    KEEP(*(.init));
    KEEP(*(.text._start));
    *(.text .text.*);
    *(.rodata .rodata.*);
  } > ROM

  .data :
  {
    *(.data .data.*);
  } > RAM

  .bss (NOLOAD) :
  {
    *(.bss .bss.*);
    *(COMMON);
  } > RAM
}