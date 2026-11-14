use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::{env, fs};

use const_gen::*;
use xz2::read::XzEncoder;

fn main() {
    const FIRMWARE_VERSION: &str = "1.0.0";

    // Forward the active protocol/board features to the rmk macro, which
    // cannot see CARGO_FEATURE_* itself (build scripts only).
    let active = std::env::vars()
        .filter_map(|(name, _)| {
            name.strip_prefix("CARGO_FEATURE_")
                .map(|feature| feature.to_lowercase())
        })
        .collect::<Vec<_>>()
        .join(",");
    println!("cargo:rustc-env=RMK_BOARD_FEATURES={active}");

    let vial_path = configured_path("VIAL_JSON_PATH", "vial.json");
    let keyboard_path = configured_path("KEYBOARD_TOML_PATH", "keyboard.toml");

    println!("cargo:rerun-if-env-changed=VIAL_JSON_PATH");
    println!("cargo:rerun-if-env-changed=KEYBOARD_TOML_PATH");
    println!("cargo:rerun-if-changed={}", vial_path.display());
    println!("cargo:rerun-if-changed={}", keyboard_path.display());
    println!("cargo:rerun-if-changed=memory.x");

    let product_id = generate_vial_config(&vial_path);
    validate_keyboard_product_id(&keyboard_path, product_id);
    println!("cargo:rustc-env=RMK_FIRMWARE_VERSION={FIRMWARE_VERSION}");

    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());

    println!("cargo:rustc-link-arg=--nmagic");
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rustc-link-arg=-Tdefmt.x");
}

fn configured_path(variable: &str, default: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn generate_vial_config(vial_path: &Path) -> u16 {
    let out_file = Path::new(&env::var_os("OUT_DIR").unwrap()).join("config_generated.rs");

    let content = fs::read_to_string(vial_path).unwrap_or_else(|e| panic!("Cannot read {}: {e}", vial_path.display()));
    let parsed = json::parse(&content).unwrap_or_else(|e| panic!("Cannot parse {}: {e}", vial_path.display()));
    let product_id = parsed["productId"]
        .as_str()
        .and_then(parse_hex_u16)
        .unwrap_or_else(|| panic!("{} productId must be a hexadecimal string", vial_path.display()));
    // The SM62 keyboard UID from the QMK firmware (keymaps/v1/config.h), so
    // Vial/Entropy keeps treating a reflashed board as the same keyboard.
    let keyboard_id: Vec<u8> = match product_id {
        0x00B2 => vec![0x13, 0xB2, 0x6E, 0x70, 0x9B, 0x42, 0x35, 0x5E],
        _ => panic!("Unsupported SM62 productId: 0x{product_id:04X}"),
    };

    let vial_cfg = json::stringify(parsed);
    let mut keyboard_def_compressed: Vec<u8> = Vec::new();
    XzEncoder::new(vial_cfg.as_bytes(), 6)
        .read_to_end(&mut keyboard_def_compressed)
        .unwrap();

    let const_declarations = [
        const_declaration!(pub VIAL_KEYBOARD_DEF = keyboard_def_compressed),
        const_declaration!(pub VIAL_KEYBOARD_ID = keyboard_id),
    ]
    .map(|s| "#[allow(clippy::redundant_static_lifetimes)]\n".to_owned() + s.as_str())
    .join("\n");
    fs::write(out_file, const_declarations).unwrap();

    product_id
}

fn validate_keyboard_product_id(keyboard_path: &Path, expected: u16) {
    let content =
        fs::read_to_string(keyboard_path).unwrap_or_else(|e| panic!("Cannot read {}: {e}", keyboard_path.display()));
    let actual = content
        .lines()
        .filter_map(|line| line.split_once('='))
        .find_map(|(key, value)| (key.trim() == "product_id").then(|| value.trim()))
        .and_then(parse_hex_u16)
        .unwrap_or_else(|| panic!("{} product_id must be a hexadecimal integer", keyboard_path.display()));

    assert_eq!(
        actual,
        expected,
        "{} and the selected Vial definition have different product IDs",
        keyboard_path.display()
    );
}

fn parse_hex_u16(value: &str) -> Option<u16> {
    let value = value.trim().trim_matches('"');
    let value = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X"))?;
    u16::from_str_radix(value, 16).ok()
}
