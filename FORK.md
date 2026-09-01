# About this fork

`AlexBSoD/rmk` is a personal fork of [`ergohaven/rmk`](https://github.com/ergohaven/rmk).
It exists to carry firmware work that is deliberately *not* meant for upstream,
plus the occasional environment fix that upstream has not taken yet. Everything
else is upstream's and should be read there.

## Branches

| Branch | Base | Purpose | Upstream? |
|--------|------|---------|-----------|
| `main` | — | Plain mirror of `ergohaven/rmk@main`. Fast-forward only, never commit here. | n/a |
| `feat/k04-agent-status` | `main` | Host coding-agent summary on the Qube dongle screen. | No — fork only |
| `screenless` | `main` | Screenless XIAO nRF52840 dongle profile, USB-only host transport. | No — fork only |
| `fix/flake-libclang` | `main` | `LIBCLANG_PATH` for the Nix dev shell, so `bindgen` builds. | Candidate |

`main` is kept byte-identical to upstream on purpose: it is the base every topic
branch rebases onto, and a fast-forward is the only update it ever needs. Local
changes go on a topic branch, including this file.

## Topic branch shape

Topic branches are kept as clean stacks rebased straight onto `main` — no merge
commits, no unrelated fixes riding along. When upstream moves, rebase and
force-push the branch; do not merge `main` into it. That way any branch can be
dropped onto any upstream state, and a branch that upstream does take applies as
a plain series of patches.

## `feat/k04-agent-status`

Three commits touching `rmk/src/host_data.rs`, the `0xB0` packet in
`rmk/src/host/via/mod.rs`, and the agent panel in
`keyboards/k04/src/qube_display.rs`. See
[`keyboards/k04/README.md`](keyboards/k04/README.md) for the screen layout, the
packet format and the feed's TTL.

This is personal daemon integration — the host side is `qubeherd`, bridging
`herdr` to the dongle — so it stays here and no PR is opened against
`ergohaven/rmk` for it.

## `screenless`

A second dongle built on a stock Seeed XIAO nRF52840 for travel, while the
Ergohaven dongle with the screen stays on the desk. Two commits: the
`RMK_DISABLE_BLE_HOST` escape hatch in `rmk-macro`, which lets a USB dongle skip
the host-facing BLE transport it has no use for, and the `qube_xiao` profile
itself in `keyboards/k04`. See [`keyboards/k04/README.md`](keyboards/k04/README.md)
for the flash layout — the XIAO bootloader keeps S140 resident, so the
application links at `0x27000` rather than `0x1000`.

The two dongles share Product ID `0x0071` and never run at the same time. A half
remembers exactly one central, so swapping dongles costs one directed
advertising window — around 30 seconds — before the half rebinds. That is
accepted rather than fixed.

## Building test firmware

Test builds are based on the topic branches, not on `main`: upstream `main` lags
the fixes actually in use, and building from it silently drops them. Merge the
relevant branches into a throwaway integration branch, build there, and record
which branches went into the image.

Build commands for every K:04 profile are in
[`keyboards/k04/README.md`](keyboards/k04/README.md).

## Contributing back

Fixes that *are* upstream-worthy follow the normal Ergohaven flow: push the
branch here, then open the PR against `ergohaven/rmk` with `AlexBSoD:<branch>`
as the head. Note that issues are disabled on the upstream repository, so a
defect report has to travel as a PR whose body carries the analysis.
