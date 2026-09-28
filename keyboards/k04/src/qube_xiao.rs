#![no_main]
#![no_std]

//! K:04 Qube dongle on a Seeed XIAO nRF52840 (Sense) — USB HID central, no screen.
//!
//! Build: `cargo make uf2-qube-xiao`

#[path = "../../common/default_layer_names.rs"]
mod default_layer_names;
mod layer_names;
mod module_settings;
mod xiao_status_led;

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
    fn status_led() -> crate::xiao_status_led::XiaoStatusLed {
        let mut config = ::embassy_nrf::pwm::SimpleConfig::default();
        config.max_duty = crate::xiao_status_led::MAX_DUTY;
        // Active-low LED: park the pins high so it stays dark when PWM is off.
        config.ch0_idle_level = ::embassy_nrf::gpio::Level::High;
        config.ch1_idle_level = ::embassy_nrf::gpio::Level::High;
        config.ch2_idle_level = ::embassy_nrf::gpio::Level::High;
        let pwm = ::embassy_nrf::pwm::SimplePwm::new_3ch(p.PWM0, p.P0_26, p.P0_30, p.P0_06, &config);
        crate::xiao_status_led::XiaoStatusLed::new(pwm)
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
