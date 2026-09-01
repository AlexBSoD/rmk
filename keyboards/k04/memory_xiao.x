MEMORY
{
  /* NOTE 1 K = 1 KiB = 1024 bytes */
  /* Seeed XIAO nRF52840 (Sense) dongle: Adafruit bootloader with S140 7.3.0 */
  /* resident at 0x00001000..0x00027000, so the application starts above it. */
  /* Reserve 0xCC000..0xEC000 for RMK storage, same as the stock Qube dongle. */
  FLASH : ORIGIN = 0x00027000, LENGTH = 660K
  RAM : ORIGIN = 0x20000008, LENGTH = 255K
}
