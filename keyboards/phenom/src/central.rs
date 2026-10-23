#![no_main]
#![no_std]

mod pointing_mode;

use rmk::macros::rmk_central;

#[rmk_central]
mod keyboard_central {
    #[register_processor]
    fn pointing_mode() -> crate::pointing_mode::PhenomPointingMode {
        crate::pointing_mode::PhenomPointingMode::new()
    }
}
