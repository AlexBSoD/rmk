#![no_main]
#![no_std]

mod battery_nrf;
#[path = "../../common/default_layer_names.rs"]
mod default_layer_names;
#[path = "../../common/layer_names.rs"]
mod layer_names;
mod trackball_processor;

const DEFAULT_LAYER_NAMES: [&str; 16] = default_layer_names::TRACKBALL;

use rmk::macros::rmk_keyboard;

#[rmk_keyboard]
mod keyboard {
    #[register_processor(event)]
    fn battery() -> crate::battery_nrf::TrackballBattery {
        crate::battery_nrf::TrackballBattery::new(p.SAADC, p.P0_31)
    }

    #[register_processor(event)]
    fn trackball_mode() -> crate::trackball_processor::TrackballModeProcessor {
        crate::trackball_processor::TrackballModeProcessor::new()
    }

    #[register_processor(poll)]
    fn ergohaven_user_keys() -> ::rmk::processor::builtin::ergohaven::ErgohavenUserKeys {
        ::rmk::processor::builtin::ergohaven::ErgohavenUserKeys::new()
    }
}
