//! Pointing modes on the Phenom, while a mode key is held: User28/29/30 put
//! both halves' module into sniper/scroll/text mode, User31-33 only the left
//! one and User34-36 only the right one. Same ids as the K:04 Qube.

use rmk::event::{ActionEvent, PointingProcessorEvent, publish_event_async};
use rmk::input_device::pointing::{CaretConfig, PointingMode, PointingModeKeyState, ScrollConfig, SniperConfig};
use rmk::macros::processor;
use rmk::types::action::Action;

/// Pointing device ids per half, as keyboard.toml numbers them: the ball, then the touchpad.
const HALF_DEVICES: [[u8; 2]; 2] = [[0, 2], [1, 3]];
/// Motion counts per arrow key press in text mode.
const TEXT_THRESHOLD: i16 = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Sniper,
    Scroll,
    Text,
}

#[processor(subscribe = [ActionEvent])]
pub struct PhenomPointingMode {
    keys: [PointingModeKeyState<Mode>; 2],
    published: [PointingMode; 2],
}

impl PhenomPointingMode {
    pub fn new() -> Self {
        Self {
            keys: [PointingModeKeyState::new(), PointingModeKeyState::new()],
            published: [PointingMode::default(); 2],
        }
    }

    async fn on_action_event(&mut self, event: ActionEvent) {
        let Action::User(id) = event.action else {
            return;
        };
        let (halves, mode) = match id {
            28 => ([true, true], Mode::Sniper),
            29 => ([true, true], Mode::Scroll),
            30 => ([true, true], Mode::Text),
            31 => ([true, false], Mode::Sniper),
            32 => ([true, false], Mode::Scroll),
            33 => ([true, false], Mode::Text),
            34 => ([false, true], Mode::Sniper),
            35 => ([false, true], Mode::Scroll),
            36 => ([false, true], Mode::Text),
            _ => return,
        };

        for half in 0..2 {
            if !halves[half] {
                continue;
            }
            self.keys[half].handle(mode, event.keyboard_event.pressed, false);
            let pointing_mode = match self.keys[half].mode_override() {
                None => PointingMode::default(),
                Some(Mode::Sniper) => PointingMode::Sniper(SniperConfig::default()),
                Some(Mode::Scroll) => PointingMode::Scroll(ScrollConfig::default()),
                Some(Mode::Text) => PointingMode::Caret(CaretConfig {
                    threshold: TEXT_THRESHOLD,
                    ..Default::default()
                }),
            };
            if pointing_mode == self.published[half] {
                continue;
            }
            for device_id in HALF_DEVICES[half] {
                publish_event_async(PointingProcessorEvent {
                    device_id,
                    mode: pointing_mode,
                })
                .await;
            }
            self.published[half] = pointing_mode;
        }
    }
}
