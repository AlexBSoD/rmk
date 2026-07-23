use const_gen::*;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::{env, fs};
use xz2::read::XzEncoder;

fn main() {
    const FIRMWARE_VERSION: &str = "0.1.2";
    const FIRMWARE_VERSION_BCD: &str = "0x0102";

    println!("cargo:rerun-if-changed=vial.json");
    println!("cargo:rerun-if-changed=keyboard.toml");
    println!("cargo:rerun-if-changed=memory_halves.x");
    println!("cargo:rerun-if-changed=memory_qube.x");
    println!("cargo:rerun-if-env-changed=VIAL_JSON_PATH");
    println!("cargo:rustc-env=RMK_FIRMWARE_VERSION={FIRMWARE_VERSION}");
    println!("cargo:rustc-env=RMK_FIRMWARE_VERSION_BCD={FIRMWARE_VERSION_BCD}");

    if env::var_os("CARGO_FEATURE_QUBE").is_some() {
        println!("cargo:rustc-env=RMK_VIAL_DEVICE_SETTINGS_FN=crate::layer_names::vial_device_settings");
    }

    generate_vial_config();

    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let memory = if env::var_os("CARGO_FEATURE_QUBE").is_some() {
        include_bytes!("memory_qube.x").as_slice()
    } else {
        include_bytes!("memory_halves.x").as_slice()
    };
    File::create(out.join("memory.x")).unwrap().write_all(memory).unwrap();
    println!("cargo:rustc-link-search={}", out.display());

    println!("cargo:rustc-link-arg=--nmagic");
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rustc-link-arg=-Tdefmt.x");
    println!("cargo:rustc-linker=flip-link");
}

fn generate_vial_config() {
    let out_file = Path::new(&env::var_os("OUT_DIR").unwrap()).join("config_generated.rs");

    let vial_path = env::var_os("VIAL_JSON_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("vial.json"));
    println!("cargo:rerun-if-changed={}", vial_path.display());

    let mut content = String::new();
    match File::open(&vial_path) {
        Ok(mut file) => {
            file.read_to_string(&mut content).expect("Cannot read vial.json");
        }
        Err(e) => panic!("Cannot find vial.json {:?}: {}", vial_path, e),
    };

    let parsed = json::parse(&content).expect("Cannot parse vial.json");
    let product_id = parsed["productId"]
        .as_str()
        .and_then(|value| value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")))
        .and_then(|value| u16::from_str_radix(value, 16).ok())
        .expect("vial.json productId must be a hexadecimal string");
    let mut vial_cfg = json::stringify(parsed);
    if !vial_cfg.contains("\"entropy\"") {
        vial_cfg.insert_str(
            1,
            "\"entropy\":{\"liveFeatures\":[\"time\",\"media\"],\"batteryHalves\":true},",
        );
    }
    let mut keyboard_def_compressed: Vec<u8> = Vec::new();
    XzEncoder::new(vial_cfg.as_bytes(), 6)
        .read_to_end(&mut keyboard_def_compressed)
        .unwrap();

    let keyboard_id: Vec<u8> = match product_id {
        // Keep the established Vial identities for compatibility with saved layouts.
        0x0071 => vec![0x80, 0x04, 0x28, 0xAB, 0x69, 0x3E, 0x19, 0x60],
        0x0072 | 0x0073 => vec![0x80, 0x04, 0x2D, 0x7A, 0x91, 0x44, 0x3B, 0x21],
        _ => panic!("Unsupported K:04 Qube productId: 0x{product_id:04X}"),
    };
    let const_declarations = [
        const_declaration!(pub VIAL_KEYBOARD_DEF = keyboard_def_compressed),
        const_declaration!(pub VIAL_KEYBOARD_ID = keyboard_id),
    ]
    .map(|s| "#[allow(clippy::redundant_static_lifetimes)]\n".to_owned() + s.as_str())
    .join("\n");
    fs::write(out_file, const_declarations).unwrap();
}
