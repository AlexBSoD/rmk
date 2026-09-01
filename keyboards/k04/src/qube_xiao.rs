#![no_main]
#![no_std]

//! K:04 Qube dongle on a Seeed XIAO nRF52840 (Sense) — USB HID central, no screen.
//!
//! Build: `cargo make uf2-qube-xiao`

#[path = "../../common/default_layer_names.rs"]
mod default_layer_names;
mod layer_names;
mod module_settings;

const DEFAULT_LAYER_NAMES: [&str; 16] = default_layer_names::STANDARD_WITH_MOUSE;

use rmk::macros::rmk_central;

#[rmk_central]
mod keyboard_central {
    #[register_processor(event)]
    fn module_settings_broadcast() -> crate::layer_names::ModuleSettingsBroadcast {
        crate::layer_names::ModuleSettingsBroadcast::new()
    }

    #[register_processor(event)]
    fn module_settings_sync() -> crate::module_settings::ModuleSettingsSync {
        crate::module_settings::ModuleSettingsSync::new()
    }

    #[register_processor(poll)]
    fn ergohaven_user_keys() -> ::rmk::processor::builtin::ergohaven::ErgohavenUserKeys {
        ::rmk::processor::builtin::ergohaven::ErgohavenUserKeys::new()
    }

    #[register_processor(poll)]
    fn pointing_processor() -> ::rmk::input_device::pointing::QubePointingModeProcessor<'static> {
        ::rmk::input_device::pointing::QubePointingModeProcessor::new(&keymap)
    }
}
