# Ergohaven K:04 Series + Qube

One Qube split BLE firmware crate for K:04, K:04 Mini, and K:04 Micro.
All Rust code is shared; each model has its own matrix, factory keymap, Vial
definition, Product ID, and Vial keyboard ID.

This target is intentionally separate from `keyboards/k04`:

- `qube` is the USB HID central/dongle with the ST7789 display.
- `left` and `right` are BLE peripherals with ids `0` and `1`.
- RMK comes from the root workspace crates (`../../rmk`, `../../rmk-types`),
  synced from official upstream `https://github.com/HaoboGu/rmk` main.

| Profile | Keyboard config | Vial config | Matrix | Product ID |
|---------|-----------------|-------------|--------|------------|
| K:04 | `keyboard.toml` | `vial.json` | 10×6 | `0x0071` |
| Mini | `keyboard_mini.toml` | `vial_mini.json` | 8×6 | `0x0072` |
| Micro | `keyboard_micro.toml` | `vial_micro.json` | 8×6 | `0x0073` |

## Build

```sh
KEYBOARD_TOML_PATH="$PWD/keyboard.toml" \
VIAL_JSON_PATH="$PWD/vial.json" \
CARGO_TARGET_DIR=target/k04/qube \
cargo build --release --bin qube --features qube

KEYBOARD_TOML_PATH="$PWD/keyboard.toml" \
VIAL_JSON_PATH="$PWD/vial.json" \
CARGO_TARGET_DIR=target/k04/halves \
cargo build --release --bin left --bin right
```

Replace the two config paths and `k04` target directory with
`keyboard_mini.toml` / `vial_mini.json` / `mini` or
`keyboard_micro.toml` / `vial_micro.json` / `micro`.

```sh
./scripts/build_k04_matrix.sh
```

## Scope

The K:04 Series + Qube target covers matrix, split BLE, two encoders, battery
telemetry, and the Qube status screen. It shares the root RMK BLE split engine
with `keyboards/k04`; only the configured central topology differs.

## Battery

This firmware does not use RMK's `battery_adc_pin` codegen path. The halves use
`src/battery_nrf.rs`, which samples `P0_31` without `calibrate().await` and
re-publishes `BatteryStatusEvent` periodically.
