#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

failures=0

fail() {
    echo "profile contract: $*" >&2
    failures=$((failures + 1))
}

toml_value() {
    local file="$1"
    local key="$2"
    awk -v key="$key" '
        $0 ~ "^[[:space:]]*" key "[[:space:]]*=" {
            value = $0
            sub(/#.*/, "", value)
            sub(/^[^=]*=/, "", value)
            gsub(/^[[:space:]"]+|[[:space:]"]+$/, "", value)
            print value
            exit
        }
    ' "$file"
}

expect_toml() {
    local file="$1"
    local key="$2"
    local expected="$3"
    local actual
    actual="$(toml_value "$file" "$key")"
    if [[ "$actual" != "$expected" ]]; then
        fail "$file: $key=$actual, expected $expected"
    fi
}

expect_not_true() {
    local file="$1"
    local key="$2"
    local actual
    actual="$(toml_value "$file" "$key")"
    if [[ "$actual" == "true" ]]; then
        fail "$file: $key must not be true in production firmware"
    fi
}

profiles=(
    keyboards/imperial44/keyboard.toml
    keyboards/k03/keyboard.toml
    keyboards/k04/keyboard.toml
    keyboards/k04/keyboard_micro.toml
    keyboards/k04/keyboard_mini.toml
    keyboards/k04_qube/keyboard.toml
    keyboards/k04_qube/keyboard_micro.toml
    keyboards/k04_qube/keyboard_mini.toml
    keyboards/op36/keyboard.toml
    keyboards/op36_qube/keyboard.toml
    keyboards/op36_qube/keyboard_imperial44.toml
    keyboards/op36_qube/keyboard_k03.toml
    keyboards/op36_qube/keyboard_velvet.toml
    keyboards/trackball_royale/keyboard.toml
    keyboards/trackball_v30/keyboard.toml
    keyboards/trackball_v31/keyboard.toml
    keyboards/velvet/keyboard.toml
    keyboards/velvet_ui/keyboard.toml
)

split_profiles=(
    keyboards/imperial44/keyboard.toml
    keyboards/k03/keyboard.toml
    keyboards/k04/keyboard.toml
    keyboards/k04/keyboard_micro.toml
    keyboards/k04/keyboard_mini.toml
    keyboards/k04_qube/keyboard.toml
    keyboards/k04_qube/keyboard_micro.toml
    keyboards/k04_qube/keyboard_mini.toml
    keyboards/op36/keyboard.toml
    keyboards/op36_qube/keyboard.toml
    keyboards/op36_qube/keyboard_imperial44.toml
    keyboards/op36_qube/keyboard_k03.toml
    keyboards/op36_qube/keyboard_velvet.toml
    keyboards/velvet/keyboard.toml
    keyboards/velvet_ui/keyboard.toml
)

standalone_split_profiles=(
    keyboards/imperial44/keyboard.toml
    keyboards/k03/keyboard.toml
    keyboards/k04/keyboard.toml
    keyboards/k04/keyboard_micro.toml
    keyboards/k04/keyboard_mini.toml
    keyboards/op36/keyboard.toml
    keyboards/velvet/keyboard.toml
    keyboards/velvet_ui/keyboard.toml
)

qube_profiles=(
    keyboards/k04_qube/keyboard.toml
    keyboards/k04_qube/keyboard_micro.toml
    keyboards/k04_qube/keyboard_mini.toml
    keyboards/op36_qube/keyboard.toml
    keyboards/op36_qube/keyboard_imperial44.toml
    keyboards/op36_qube/keyboard_k03.toml
    keyboards/op36_qube/keyboard_velvet.toml
)

non_k04_profiles=(
    keyboards/imperial44/keyboard.toml
    keyboards/k03/keyboard.toml
    keyboards/op36/keyboard.toml
    keyboards/op36_qube/keyboard.toml
    keyboards/op36_qube/keyboard_imperial44.toml
    keyboards/op36_qube/keyboard_k03.toml
    keyboards/op36_qube/keyboard_velvet.toml
    keyboards/trackball_royale/keyboard.toml
    keyboards/trackball_v30/keyboard.toml
    keyboards/trackball_v31/keyboard.toml
    keyboards/velvet/keyboard.toml
    keyboards/velvet_ui/keyboard.toml
)

for file in "${profiles[@]}"; do
    expect_toml "$file" manufacturer Ergohaven
    expect_toml "$file" layers 16
    expect_toml "$file" combo_max_num 32
    expect_toml "$file" morse_max_num 32
    expect_toml "$file" macro_space_size 2048
    expect_toml "$file" ble_profiles_num 5
    expect_toml "$file" ble_reconnect_timeout_seconds 60
    expect_toml "$file" ble_pairing_timeout_seconds 60
    expect_not_true "$file" clear_storage
    expect_not_true "$file" clear_layout
done

for file in "${non_k04_profiles[@]}"; do
    expect_toml "$file" combo_max_length 4
    expect_toml "$file" fork_max_num 8
    expect_toml "$file" max_patterns_per_key 8
    expect_toml "$file" protocol_max_bulk_size 8
    expect_toml "$file" protocol_macro_chunk_size 64
done

for file in "${split_profiles[@]}"; do
    expect_toml "$file" split_pairing_timeout_seconds 30
done

for file in "${standalone_split_profiles[@]}"; do
    expect_toml "$file" split_central_sleep_timeout_seconds 120
done

for file in "${qube_profiles[@]}"; do
    expect_toml "$file" split_central_sleep_timeout_seconds 900
done

for file in "${profiles[@]}"; do
    expect_toml "$file" start_addr 0xCC000
    expect_toml "$file" num_sectors 32
done

memory_files=(
    keyboards/imperial44/memory.x
    keyboards/k03/memory.x
    keyboards/k04/memory.x
    keyboards/k04_qube/memory_halves.x
    keyboards/k04_qube/memory_qube.x
    keyboards/op36/memory.x
    keyboards/op36_qube/memory_halves.x
    keyboards/op36_qube/memory_qube.x
    keyboards/trackball_royale/memory.x
    keyboards/trackball_v30/memory.x
    keyboards/trackball_v31/memory.x
    keyboards/velvet/memory.x
    keyboards/velvet_ui/memory.x
)
for file in "${memory_files[@]}"; do
    rg -Fq 'Reserve 0xCC000..0xEC000 for RMK storage.' "$file" \
        || fail "$file: unified storage reservation is missing"
    rg -q 'FLASH[[:space:]]*:[[:space:]]*ORIGIN[[:space:]]*=[[:space:]]*0x000(26000|01000),[[:space:]]*LENGTH[[:space:]]*=[[:space:]]*(664|812)K' "$file" \
        || fail "$file: application linker must stop at 0xCC000"
done

mapfile -t build_scripts < <(git ls-files 'keyboards/*/build.rs')
for file in "${build_scripts[@]}"; do
    rg -q 'const FIRMWARE_VERSION: &str = "0\.1\.3";' "$file" \
        || fail "$file: firmware version must be 0.1.3"
    rg -q 'const FIRMWARE_VERSION_BCD: &str = "0x0103";' "$file" \
        || fail "$file: BCD firmware version must be 0x0103"
done

mapfile -t vial_definitions < <(git ls-files 'keyboards/*/vial*.json')
for file in "${vial_definitions[@]}"; do
    jq -e '.manufacturer == "Ergohaven"' "$file" >/dev/null \
        || fail "$file: manufacturer must be Ergohaven"
    jq -e '.firmware.version == "0.1.3" and .firmwareVersion == "0.1.3"' "$file" >/dev/null \
        || fail "$file: both firmware versions must be 0.1.3"
done

for file in "${vial_definitions[@]}"; do
    if [[ "$file" != keyboards/trackball_* ]]; then
        jq -e '.entropy.batteryHalves == true' "$file" >/dev/null \
            || fail "$file: split devices must advertise entropy.batteryHalves"
    fi
done

for file in keyboards/op36_qube/vial.json keyboards/k04_qube/vial{,_mini,_micro}.json; do
    jq -e '
        .entropy.batteryHalves == true
        and (.entropy.liveFeatures | index("time") != null)
        and (.entropy.liveFeatures | index("media") != null)
    ' "$file" >/dev/null || fail "$file: Qube must advertise time, media, and half batteries"
done

default_names_source=keyboards/common/default_layer_names.rs
python3 - "$default_names_source" <<'PY' || fail "$default_names_source: factory layer-name profiles drifted"
import ast
import re
import sys

source = open(sys.argv[1], encoding="utf-8").read()

def rust_array(name):
    match = re.search(
        rf"pub const {name}:.*?=\s*\[(.*?)\];",
        source,
        flags=re.DOTALL,
    )
    if not match:
        raise SystemExit(f"missing {name}")
    return ast.literal_eval("[" + match.group(1) + "]")

numeric_tail = [str(index) for index in range(5, 16)]
assert rust_array("STANDARD_NO_MOUSE") == [
    "Base", "Navigation", "Symbols", "Adjust", "4", *numeric_tail
]
assert rust_array("STANDARD_WITH_MOUSE") == [
    "Base", "Navigation", "Symbols", "Adjust", "Mouse", *numeric_tail
]
assert rust_array("TRACKBALL") == [
    "Mouse", "1", "2", "Adjust", "4", *numeric_tail
]
PY

standard_no_mouse_roots=(
    keyboards/imperial44/src/central.rs
    keyboards/k03/src/central.rs
    keyboards/op36/src/central.rs
    keyboards/velvet/src/central.rs
)
for file in "${standard_no_mouse_roots[@]}"; do
    rg -Fq 'default_layer_names::STANDARD_NO_MOUSE' "$file" \
        || fail "$file: standard no-Mouse layer names are missing"
done

standard_with_mouse_roots=(
    keyboards/k04/src/central.rs
    keyboards/k04_qube/src/qube.rs
    keyboards/velvet_ui/src/central.rs
)
for file in "${standard_with_mouse_roots[@]}"; do
    rg -Fq 'default_layer_names::STANDARD_WITH_MOUSE' "$file" \
        || fail "$file: standard Mouse layer names are missing"
done

for file in keyboards/trackball_{royale,v30,v31}/src/keyboard.rs; do
    rg -Fq 'default_layer_names::TRACKBALL' "$file" \
        || fail "$file: functional trackball layer names are missing"
done

rg -Fq 'crate::default_layer_names::STANDARD_NO_MOUSE' keyboards/op36_qube/build.rs \
    || fail "keyboards/op36_qube/build.rs: generated Qube defaults drifted"
rg -Fq 'const STORAGE_VERSION: u8 = 2;' keyboards/common/layer_names.rs \
    || fail "keyboards/common/layer_names.rs: default-name migration version drifted"
for file in keyboards/{k04,k04_qube}/src/layer_names.rs; do
    rg -Fq 'const STORAGE_VERSION: u8 = 3;' "$file" \
        || fail "$file: K:04 default-name migration version drifted"
    rg -Fq 'migrate_legacy_placeholders();' "$file" \
        || fail "$file: generated layer-name migration is missing"
done

classic_user_registry='BT0,BT1,BT2,BT3,BT4,BT_NEXT,BT_PREV,BT_CLR,BT_TOG,BT_PEER'
for file in \
    keyboards/imperial44/vial.json \
    keyboards/k03/vial.json \
    keyboards/op36/vial.json \
    keyboards/trackball_royale/vial.json \
    keyboards/trackball_v30/vial.json \
    keyboards/trackball_v31/vial.json \
    keyboards/velvet/vial.json \
    keyboards/velvet_ui/vial.json
do
    actual="$(jq -r '.customKeycodes[0:10] | map(.name) | join(",")' "$file")"
    if [[ "$actual" != "$classic_user_registry" ]]; then
        fail "$file: classic USER00..USER09 registry drifted"
    fi
done

reset_source=tools/settings_reset/src/main.rs
rg -Fq 'const STORAGE_RANGE: (u32, u32) = (0xCC000, 0xEC000);' "$reset_source" \
    || fail "$reset_source: unified storage reset range drifted"
rg -q '0xA0000|0xC0000|ERASE_RANGES' "$reset_source" \
    && fail "$reset_source: reset must erase only the unified storage partition"

migration_source=tools/storage_migrate/src/main.rs
rg -Fq 'const LEGACY_START: u32 = 0xA0000;' "$migration_source" \
    || fail "$migration_source: legacy source address drifted"
rg -Fq 'const UNIFIED_START: u32 = 0xCC000;' "$migration_source" \
    || fail "$migration_source: unified destination address drifted"
rg -Fq 'fn destination_is_safe() -> bool' "$migration_source" \
    || fail "$migration_source: destination safety preflight is missing"

for tool in settings_reset storage_migrate; do
    if [[ -e "tools/$tool/memory.x" ]]; then
        fail "tools/$tool/memory.x: source file would shadow the generated Qube linker script"
    fi
    rg -Fq 'include_bytes!("memory_halves.x")' "tools/$tool/build.rs" \
        || fail "tools/$tool/build.rs: halves linker selection is missing"
    rg -Fq 'include_bytes!("memory_qube.x")' "tools/$tool/build.rs" \
        || fail "tools/$tool/build.rs: Qube linker selection is missing"
    rg -q 'FLASH[[:space:]]*:[[:space:]]*ORIGIN[[:space:]]*=[[:space:]]*0x00026000' "tools/$tool/memory_halves.x" \
        || fail "tools/$tool/memory_halves.x: application origin must be 0x26000"
    rg -q 'FLASH[[:space:]]*:[[:space:]]*ORIGIN[[:space:]]*=[[:space:]]*0x00001000' "tools/$tool/memory_qube.x" \
        || fail "tools/$tool/memory_qube.x: application origin must be 0x1000"
done

if ((failures > 0)); then
    echo "Ergohaven firmware profile contract failed with $failures error(s)." >&2
    exit 1
fi

echo "Ergohaven firmware profile contract OK (${#profiles[@]} production profiles)."
