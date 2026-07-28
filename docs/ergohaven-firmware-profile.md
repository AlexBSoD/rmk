# Ergohaven Firmware Profile

K:04 Series is the reference profile for Ergohaven RMK firmware. Production
profiles share this software contract:

- 16 layers
- 32 combos, up to 4 keys each
- 32 Morse / Tap Dance entries, up to 8 patterns each
- 2048 bytes of macro storage
- 8 forks
- 5 Bluetooth profiles
- protocol bulk size 8 and macro chunk size 64 bytes
- Bluetooth reconnect and pairing windows of 60 seconds
- persistent layer names at QSID 200–215, up to 12 UTF-8 bytes each
- firmware version 0.1.3 and manufacturer `Ergohaven`

Split pairing uses a 30-second window. Standalone split centrals sleep after
120 seconds; powered Qube centrals use 900 seconds. Entropy capability metadata
advertises separate half batteries on all split devices and time/media live
features only where Qube supplies them.

Hardware-specific differences remain in matrices, pins, encoder count,
pointing devices, displays, lighting, batteries, and split topology.

## USER keycode compatibility

Classic firmware keeps the established USER00–USER09 numeric mapping so saved
layouts remain valid. The names are standardized as `BT0`–`BT4`, `BT_NEXT`,
`BT_PREV`, `BT_CLR`, `BT_TOG`, and `BT_PEER`.

K:04 retains its established numeric mapping and its larger named USER set.
RMK's Ergohaven processor accepts both mappings. Renumbering either mapping
requires an explicit saved-layout migration and is outside this compatibility
stage.

## Storage and reset

All production nRF52840 profiles reserve the same 128 KiB settings partition
at `0xCC000–0xEC000`. Application linkers stop at `0xCC000`, preventing
firmware and settings from overlapping.

Use `settings_reset.uf2` for halves and standalone trackballs, and
`settings_reset_qube.uf2` for a Qube dongle. Both erase only the unified
partition; separate files are required because the application origins differ.

Profiles upgrading from the legacy `0xA0000–0xC0000` partition can preserve
their raw settings by running the matching one-time storage migration utility
before flashing the new firmware.

Run `./scripts/check_ergohaven_profile.sh` to reject accidental profile drift.
