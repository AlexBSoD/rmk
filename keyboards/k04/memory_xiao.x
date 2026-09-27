MEMORY
{
  /* NOTE 1 K = 1 KiB = 1024 bytes */
  /* Seeed XIAO nRF52840 (Sense) dongle: Adafruit bootloader, no SoftDevice. */
  /* The application starts right above the MBR, like the stock Qube dongle; */
  /* flashing it over a factory S140 erases the SoftDevice, which RMK never uses. */
  /* Reserve 0xCC000..0xEC000 for RMK storage, same as the stock Qube dongle. */
  FLASH : ORIGIN = 0x00001000, LENGTH = 812K
  RAM : ORIGIN = 0x20000008, LENGTH = 255K
}
