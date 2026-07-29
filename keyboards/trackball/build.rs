use const_gen::*;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::{env, fs};
use xz2::read::XzEncoder;

fn main() {
    const FIRMWARE_VERSION: &str = "0.1.3";
    const FIRMWARE_VERSION_BCD: &str = "0x0103";

    let vial_path = configured_path("VIAL_JSON_PATH", "vial_mini_v31.json");
    let keyboard_path = configured_path("KEYBOARD_TOML_PATH", "keyboard_mini_v31.toml");

    println!("cargo:rerun-if-env-changed=VIAL_JSON_PATH");
    println!("cargo:rerun-if-env-changed=KEYBOARD_TOML_PATH");
    println!("cargo:rerun-if-changed={}", vial_path.display());
    println!("cargo:rerun-if-changed={}", keyboard_path.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rustc-env=RMK_FIRMWARE_VERSION={FIRMWARE_VERSION}");
    println!("cargo:rustc-env=RMK_FIRMWARE_VERSION_BCD={FIRMWARE_VERSION_BCD}");
    println!("cargo:rustc-env=RMK_VIAL_DEVICE_SETTINGS_FN=crate::layer_names::vial_device_settings");
    println!("cargo:rustc-check-cfg=cfg(trackball_mini_v30)");
    println!("cargo:rustc-check-cfg=cfg(trackball_mini_v31)");
    println!("cargo:rustc-check-cfg=cfg(trackball_royale)");

    let product_id = generate_vial_config(&vial_path);
    validate_keyboard_product_id(&keyboard_path, product_id);
    configure_profile(product_id);

    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());

    println!("cargo:rustc-link-arg=--nmagic");
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rustc-link-arg=-Tdefmt.x");
    println!("cargo:rustc-linker=flip-link");
}

fn configured_path(variable: &str, default: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn generate_vial_config(vial_path: &Path) -> u16 {
    let out_file = Path::new(&env::var_os("OUT_DIR").unwrap()).join("config_generated.rs");

    let mut content = String::new();
    File::open(vial_path)
        .unwrap_or_else(|e| panic!("Cannot find {}: {e}", vial_path.display()))
        .read_to_string(&mut content)
        .unwrap_or_else(|e| panic!("Cannot read {}: {e}", vial_path.display()));

    let parsed = json::parse(&content).unwrap_or_else(|e| panic!("Cannot parse {}: {e}", vial_path.display()));
    let product_id = parsed["productId"]
        .as_str()
        .and_then(parse_hex_u16)
        .unwrap_or_else(|| panic!("{} productId must be a hexadecimal string", vial_path.display()));
    validate_product_id(product_id, vial_path);

    let vial_cfg = json::stringify(parsed);
    let mut keyboard_def_compressed: Vec<u8> = Vec::new();
    XzEncoder::new(vial_cfg.as_bytes(), 6)
        .read_to_end(&mut keyboard_def_compressed)
        .unwrap();

    // Preserve the existing Vial identity during this structural refactor.
    let keyboard_id: Vec<u8> = vec![0xB9, 0xBC, 0x09, 0xB2, 0x9D, 0x37, 0x4C, 0xEA];
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
        "{} and selected Vial definition have different product IDs",
        keyboard_path.display()
    );
}

fn parse_hex_u16(value: &str) -> Option<u16> {
    let value = value.trim().trim_matches('"');
    let value = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X"))?;
    u16::from_str_radix(value, 16).ok()
}

fn validate_product_id(product_id: u16, path: &Path) {
    match product_id {
        0x00C1 | 0x00C2 | 0x00C3 => {}
        _ => panic!(
            "Unsupported Ergohaven trackball productId 0x{product_id:04X} in {}",
            path.display()
        ),
    }
}

fn configure_profile(product_id: u16) {
    let profile = match product_id {
        0x00C1 => "trackball_mini_v30",
        0x00C2 => "trackball_mini_v31",
        0x00C3 => "trackball_royale",
        _ => unreachable!(),
    };
    println!("cargo:rustc-cfg={profile}");
}
