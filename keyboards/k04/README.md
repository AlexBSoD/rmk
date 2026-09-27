# Ergohaven K:04 Series

One firmware crate for K:04, K:04 Mini, and K:04 Micro in both Standalone and
Qube topologies. Module settings, pointing devices, battery reader, layer
names, dependencies, and build logic are shared. Connection topology is still
selected at compile time by the binary and profile.

- Standalone: `central` is the left half with a local matrix; `peripheral` is
  the right half.
- Qube: `qube` is the matrix-less USB dongle; `left` and `right` are BLE
  peripherals.

Each topology and model keeps its own matrix, factory keymap, Vial definition,
Product ID, Vial keyboard ID, storage, and UF2 artifact.

| Topology | Profile | Keyboard config | Vial config | Matrix | Product ID |
|----------|---------|-----------------|-------------|--------|------------|
| Standalone | K:04 | `keyboard.toml` | `vial.json` | 10×6 | `0x0074` |
| Standalone | Mini | `keyboard_mini.toml` | `vial_mini.json` | 8×6 | `0x0075` |
| Standalone | Micro | `keyboard_micro.toml` | `vial_micro.json` | 8×6 | `0x0076` |
| Qube | K:04 | `keyboard_qube.toml` | `vial_qube.json` | 10×6 | `0x0071` |
| Qube | Mini | `keyboard_qube_mini.toml` | `vial_qube_mini.json` | 8×6 | `0x0072` |
| Qube | Micro | `keyboard_qube_micro.toml` | `vial_qube_micro.json` | 8×6 | `0x0073` |

## Build

```sh
KEYBOARD_TOML_PATH="$PWD/keyboard.toml" \
VIAL_JSON_PATH="$PWD/vial.json" \
CARGO_TARGET_DIR=target/k04 \
cargo build --release --bin central --bin peripheral --bin hardreset --features production_v30g

KEYBOARD_TOML_PATH="$PWD/keyboard_mini.toml" \
VIAL_JSON_PATH="$PWD/vial_mini.json" \
CARGO_TARGET_DIR=target/mini \
cargo build --release --bin central --bin peripheral --bin hardreset --features production_v30g

KEYBOARD_TOML_PATH="$PWD/keyboard_micro.toml" \
VIAL_JSON_PATH="$PWD/vial_micro.json" \
CARGO_TARGET_DIR=target/micro \
cargo build --release --bin central --bin peripheral --bin hardreset --features production_v30g
```

Qube K:04:

```sh
KEYBOARD_TOML_PATH="$PWD/keyboard_qube.toml" \
VIAL_JSON_PATH="$PWD/vial_qube.json" \
CARGO_TARGET_DIR=target/qube/k04/dongle \
cargo build --release --bin qube --no-default-features --features qube

KEYBOARD_TOML_PATH="$PWD/keyboard_qube.toml" \
VIAL_JSON_PATH="$PWD/vial_qube.json" \
CARGO_TARGET_DIR=target/qube/k04/halves \
cargo build --release --bin left --bin right --no-default-features --features qube-half
```

Use the matching `*_mini` or `*_micro` pair for the other Qube models.
`--no-default-features` keeps Qube's USB-log backend separate from the
Standalone `defmt` backend.

The repository build matrix builds all six profiles:

```sh
./scripts/build_k04_matrix.sh
```

## Battery

The halves use `src/battery_nrf.rs`, which samples `P0_31` without
`calibrate().await` and re-publishes `BatteryStatusEvent` periodically.

## XIAO nRF52840 dongle (screenless)

`src/qube_xiao.rs` is the same Qube central without the ST7789 processor, built
for a stock Seeed XIAO nRF52840 / XIAO nRF52840 Sense. It keeps the Qube K:04
identity (`0x0071`, `vial_qube.json`, `keyboard_qube.toml`), so the halves pair
with it and Vial sees the dongle it already knows — but the two dongles must not
be plugged in at the same time.

The board ships with the Adafruit UF2 bootloader and S140 7.3.0 at
`0x1000..0x27000`, but RMK brings its own BLE controller and never calls the
SoftDevice. `memory_xiao.x` therefore links the application at `0x1000` with
812 KiB, exactly like the stock Qube dongle. On the board this was found on,
an earlier `0x1000` image had already overwritten S140: the bootloader reports
`SoftDevice: not found` in `INFO_UF2.TXT` and starts the application at
`0x1000`. Storage, bonds and the keymap live at
`0xCC000`, the same addresses as on the stock dongle.

An earlier revision linked the application at `0x27000`, above S140. That only
works while S140 is intact: once any `0x1000` image has been flashed, the
bootloader no longer finds the SoftDevice, jumps to `0x1000` and never reaches
an image at `0x27000` — the dongle stays dark on USB, and the halves look as if
they cannot connect. Check `INFO_UF2.TXT` and the first block address of a UF2
before blaming the firmware.

```sh
KEYBOARD_TOML_PATH="$PWD/keyboard_qube.toml" \
VIAL_JSON_PATH="$PWD/vial_qube.json" \
CARGO_TARGET_DIR=target/qube/xiao/dongle \
cargo build --release --bin qube_xiao --no-default-features --features qube-xiao
```

`cargo make uf2-qube-xiao` produces `firmware/k04-qube-xiao.uf2`; flash it by
double-tapping reset and copying the file onto the `XIAO-SENSE` drive.

The XIAO profile also drops the host-facing BLE transport. `build.rs` sets
`RMK_DISABLE_BLE_HOST=1` for it, and `transport_setup` in `rmk-macro` then
builds the USB transport alone (`entry.rs`). The BLE stack itself stays — the
split links need it — but the dongle no longer advertises as a pairable
keyboard, so no host can bond with the dongle instead of the keyboard. That is
not a theoretical concern: a half that stays bonded to the host gets grabbed by
it and never reaches the dongle.

`qube-xiao` deliberately leaves `rmk/usb_log` out — the CDC log backend costs
~97 KiB and there is no screen to explain what it would be logging. Build with
`--features qube-xiao-log` when a board needs debugging.

| Build | Binary size | Fits in 812 KiB |
|-------|-------------|-----------------|
| `qube-xiao` | ~440 KiB | yes, ~370 KiB spare |
| `qube-xiao-log` | ~530 KiB | yes, ~280 KiB spare |

There is no screen, no NeoPixel and no battery reader on this board: the three
discrete LEDs are not WS2812, so `layer_led.rs` is not reused and the dongle
runs without any indicator.
