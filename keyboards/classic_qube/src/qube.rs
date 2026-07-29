#![no_main]
#![no_std]

//! Ergohaven Qube dongle — USB HID central + ST7789 status screen.
//!
//! Build: `cargo make uf2-qube`

#[path = "../../common/default_layer_names.rs"]
mod default_layer_names;
#[path = "../../common/layer_names.rs"]
mod layer_names;
#[cfg(velvet_pointing)]
#[path = "../../common/velvet_pointing_mode.rs"]
mod pointing_mode;
mod qube_display;

include!(concat!(env!("OUT_DIR"), "/qube_profile_generated.rs"));

use rmk::macros::rmk_central;

#[rmk_central]
mod keyboard_central {
    add_interrupt! {
        SPIM3 => ::embassy_nrf::spim::InterruptHandler<::embassy_nrf::peripherals::SPI3>;
    }

    #[register_processor(event)]
    fn display_processor() -> crate::qube_display::DongleScreen<Irqs> {
        crate::qube_display::create_processor(
            p.SPI3, p.P1_11, p.P1_10, p.P1_13, p.P0_28, p.P0_03, p.P0_02, Irqs,
        )
    }

    #[cfg(velvet_pointing)]
    #[register_processor(event)]
    fn pointing_mode() -> crate::pointing_mode::VelvetPointingMode {
        crate::pointing_mode::VelvetPointingMode::new()
    }

    #[register_processor(poll)]
    fn ergohaven_user_keys() -> ::rmk::processor::builtin::ergohaven::ErgohavenUserKeys {
        ::rmk::processor::builtin::ergohaven::ErgohavenUserKeys::new()
    }
}
