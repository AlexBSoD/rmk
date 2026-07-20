# Ergohaven K:04

No-Qube split BLE firmware for the ordinary K:04 halves.

The regular K:04 and K:04 + Qube firmwares use the same BLE split connection
engine. Their `keyboard.toml` files differ only in topology: the regular K:04
uses the left half as a central with a local matrix and one peripheral, while
Qube is a central without a matrix and connects to two peripheral halves.

## Build

```sh
cargo build --release --bin central --bin peripheral
```

The repository build matrix also builds this target:

```sh
./scripts/build_k04_matrix.sh
```
