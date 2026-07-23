# Ergohaven K:04 Series

One no-Qube split BLE firmware crate for K:04, K:04 Mini, and K:04 Micro.
All Rust code is shared; each model has its own matrix, factory keymap, Vial
definition, Product ID, and Vial keyboard ID.

| Profile | Keyboard config | Vial config | Matrix | Product ID |
|---------|-----------------|-------------|--------|------------|
| K:04 | `keyboard.toml` | `vial.json` | 10×6 | `0x0074` |
| Mini | `keyboard_mini.toml` | `vial_mini.json` | 8×6 | `0x0075` |
| Micro | `keyboard_micro.toml` | `vial_micro.json` | 8×6 | `0x0076` |

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

The repository build matrix builds all three profiles:

```sh
./scripts/build_k04_matrix.sh
```
