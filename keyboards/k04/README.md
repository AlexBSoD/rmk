# Ergohaven K:04

No-Qube split BLE firmware for the ordinary K:04 halves.

This target uses the root RMK crates with the `common` split BLE backend:

```toml
[split]
connection = "ble"
backend = "common"
```

Keep this backend separate from `keyboards/k04_qube`, which uses `backend = "qube"` for the dongle connection flow.

## Build

```sh
cargo build --release --bin central --bin peripheral
```

The repository build matrix also builds this target:

```sh
./scripts/build_k04_matrix.sh
```
