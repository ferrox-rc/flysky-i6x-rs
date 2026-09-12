MEMORY
{
  /* 128 KB On-chip Flash memory */
  FLASH (rx)  : ORIGIN = 0x08000000, LENGTH = 128K

  /* 16 KB On-chip SRAM */
  RAM   (xrw) : ORIGIN = 0x20000000, LENGTH = 16K
}

/* Reserve stack at the top of RAM */
_stack_start = ORIGIN(RAM) + LENGTH(RAM);
