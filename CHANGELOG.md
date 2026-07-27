# Changelog

## v0.1.3

### Features

- Added complete K:04 Series firmware for K:04, Mini, and Micro in standalone and Qube configurations
- Added shared Qube firmware profiles for K:03, Velvet, Imperial44, and OP36 while preserving each model's matrix, encoders, Vial identity, and default keymap
- Added left/right battery telemetry, Qube live display data, module-aware pointing settings, configurable encoder steps, and factory-enabled touchpad acceleration and gestures
- Embedded the Ergohaven manufacturer and firmware version `0.1.3` in every Vial definition and exposed the same version through VIA `id_firmware_version`

### Fixes

- Fixed split wake latency after idle while retaining the power-saving connection interval
- Fixed excessive idle polling for trackball and touchpad modules
- Fixed BLE Vial report framing, host-session responsiveness, discovery compatibility, and split battery updates
- Fixed K:04 settings persistence, Layer LED writes, touch gestures, and Qube pointing runtime parity
