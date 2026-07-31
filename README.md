# Ergohaven RMK Firmware

RMK BLE split firmware for Ergohaven keyboards and trackballs (nRF52840).

## Supported Devices

### Keyboards (BLE split)

| Keyboard    | Layout         | Encoders | Trackball |
|-------------|----------------|----------|-----------|
| K:03        | 5×6 + 5 thumb  | 3+3      | —         |
| K:04 Series | K:04 / Mini / Micro | 1+1 | —         |
| K:04 Series + Qube | K:04 / Mini / Micro | 1+1 | Qube dongle + ST7789 |
| Imperial44  | 4×6 + 3 thumb  | 1+1      | —         |
| OP36        | 3×5 + 3 thumb  | —        | —         |
| Classic splits + Qube | K:03 / Velvet / Imperial44 / OP36 | model-specific | Qube dongle + ST7789 |
| Velvet      | 4×6 + 5 thumb  | —        | Optional PMW3610 in place of the right thumb key |

### Trackballs (standalone BLE)

| Device              | Buttons | Modes                          |
|---------------------|---------|--------------------------------|
| Trackball Royale     | 6       | Normal, Scroll, Sniper, Adjust |
| Trackball Mini v3.1 | 4       | Normal, Scroll, Sniper, Adjust |
| Trackball Mini v3.0 | 2       | Normal, Scroll, Sniper, Adjust |

### Tools

| Tool           | Description                              |
|----------------|------------------------------------------|
| settings_reset | Erases keymap and BLE bonds, resets to bootloader |
| storage_migrate | One-time legacy-to-unified storage copy |

## Building

```sh
cd keyboards/k03
cargo build --release --bin central
cargo build --release --bin peripheral
```

Standalone trackballs share `keyboards/trackball`. Select a model by passing
the matching `KEYBOARD_TOML_PATH` and `VIAL_JSON_PATH`; CI and the regression
matrix build Mini v3.0, Mini v3.1, and Royale as separate firmware artifacts.

Current K:04/OP36 regression matrix:

```sh
./scripts/build_k04_matrix.sh
```

The shared production limits and metadata are documented in
[`docs/ergohaven-firmware-profile.md`](docs/ergohaven-firmware-profile.md) and
checked with:

```sh
./scripts/check_ergohaven_profile.sh
```

## Flashing

1. Put device into bootloader (double-tap reset)
2. Copy `.uf2` file to the mounted USB drive
3. For split keyboards: flash central and peripheral separately

## Settings Reset

Flash `settings_reset.uf2` on halves and standalone trackballs, or
`settings_reset_qube.uf2` on a Qube dongle, to erase saved keymap/BLE data.
Then re-flash the normal firmware.

Non-K:04 devices upgrading from firmware that used the legacy storage address
can first run `storage_migrate.uf2`, or `storage_migrate_qube.uf2` on Qube, to
preserve the raw keymap, settings, and BLE bonds.

The first upgrade from the former Velvet UI firmware requires
`settings_reset.uf2` on both halves. Afterwards remove the old Velvet UI
Bluetooth device on the host and pair the unified Velvet firmware again.
Existing standard Velvet installations do not need this reset.

## CI

Every push builds all devices in parallel via GitHub Actions. UF2 artifacts available as build downloads.

## Releases

Packaged UF2 firmware is published on the
[GitHub Releases](https://github.com/ergohaven/rmk/releases) page. The current
firmware release is `v0.1.4`.

## RMK Version

Based on [RMK](https://github.com/HaoboGu/rmk) 0.8.2 with nRF52840 BLE support.

The root `rmk`, `rmk-macro`, `rmk-types`, and `rmk-config` crates are the
source of truth for firmware targets in this repository. All six K:04 Series
profiles live in `keyboards/k04`: Standalone and Qube builds for K:04, Mini,
and Micro. The selected binary and TOML profile keep the connection topologies
separate: a central half with a local matrix and one peripheral, or a
matrix-less Qube central with two peripheral halves.
