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
cargo build --release --bin central --bin peripheral --bin hardreset

KEYBOARD_TOML_PATH="$PWD/keyboard_mini.toml" \
VIAL_JSON_PATH="$PWD/vial_mini.json" \
CARGO_TARGET_DIR=target/mini \
cargo build --release --bin central --bin peripheral --bin hardreset

KEYBOARD_TOML_PATH="$PWD/keyboard_micro.toml" \
VIAL_JSON_PATH="$PWD/vial_micro.json" \
CARGO_TARGET_DIR=target/micro \
cargo build --release --bin central --bin peripheral --bin hardreset
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

## Host agent status (Qube screen)

The Qube dongle's ST7789 screen is laid out in three fixed zones over the
280×240 panel (`src/qube_display.rs`):

| Zone | Content |
|------|---------|
| `y 14..42` | header — host clock on the left, active layer name on the right |
| `y 50..232`, `x 18` / `x 234` | per-half battery gauges, left column = left half |
| `y 50..232`, centre | host agent summary, the reason to glance at the screen |

The agent panel is a 2×2 grid — `WORKING`, `BLOCKED` on the top row, `IDLE`,
`DONE` below — each quadrant showing a label and a count. `BLOCKED` is the only
state worth interrupting typing for, so it keeps the second slot and a distinct
colour. While no counts are known the panel renders `NO AGENT FEED` instead.

Counts come from the host, not from the firmware. A daemon pushes them over the
existing raw-HID (Via) OUT endpoint as a 32-byte packet handled by
`process_host_data_packet` in `rmk/src/host/via/mod.rs`:

| Byte | Meaning |
|------|---------|
| `0` | `0xB0` — `HOST_DATA_AGENTS` |
| `1` | payload version, currently `0x01` |
| `2..7` | counts: working, idle, blocked, done, unknown |
| `7` | reserved flags |

A packet whose version byte is unknown is swallowed rather than answered, so a
newer daemon can never have its data mistaken for a Via command. Packets sit
alongside the other `HOST_DATA_*` kinds (`0xAA` time, `0xAC` layout, `0xAD` /
`0xAE` media) and land in `rmk/src/host_data.rs`, which keeps them in RAM only —
nothing here is persisted to flash.

The summary carries a 30-second TTL (`AGENTS_TTL`). A daemon that dies or loses
the socket it watches would otherwise leave the screen advertising "1 working"
forever; past the TTL `snapshot()` reports `None` and the panel falls back to
`NO AGENT FEED`.

The host side of this lives in a separate personal project (`qubeherd`, bridging
`herdr` to the dongle) and is not part of this repository.
