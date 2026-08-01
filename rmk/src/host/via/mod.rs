use byteorder::{BigEndian, ByteOrder, LittleEndian};
use embassy_time::Instant;
use postcard::experimental::max_size::MaxSize;
use rmk_types::action::KeyAction;
use rmk_types::battery::BatteryStatus;
use rmk_types::protocol::vial::{VIA_PROTOCOL_VERSION, ViaCommand, ViaKeyboardInfo};
use vial::process_vial;

use crate::channel::{HOST_REQUEST_CHANNEL, try_send_host_reply};
use crate::config::{RmkConfig, VialConfig};
use crate::core_traits::Runnable;
use crate::hid::ViaReport;
use crate::host::context::KeyboardContext;
use crate::host::via::keycode_convert::{from_via_keycode, to_via_keycode};
use crate::{MACRO_SPACE_SIZE, boot};

pub(crate) mod keycode_convert;
mod vial;
#[cfg(feature = "vial_lock")]
mod vial_lock;

const HOST_DATA_TIME: u8 = 0xAA;
const HOST_DATA_VOLUME: u8 = 0xAB;
const HOST_DATA_LAYOUT: u8 = 0xAC;
const HOST_DATA_MEDIA_ARTIST: u8 = 0xAD;
const HOST_DATA_MEDIA_TITLE: u8 = 0xAE;
const ERGOHAVEN_CUSTOM_NAMESPACE: u8 = 0xE8;
const ERGOHAVEN_CUSTOM_BATTERY_HALVES: u8 = 0x01;
const ERGOHAVEN_BATTERY_HALVES_VERSION: u8 = 0x01;
const ERGOHAVEN_CUSTOM_NATIVE_KEY_ACTION_CAPS: u8 = 0x02;
const ERGOHAVEN_CUSTOM_NATIVE_KEY_ACTION: u8 = 0x03;
const ERGOHAVEN_CUSTOM_NEXT_NATIVE_KEY_ACTION: u8 = 0x04;
const ERGOHAVEN_NATIVE_KEY_ACTION_VERSION: u8 = 0x01;
const ERGOHAVEN_NATIVE_KEY_ACTION_CAP_GET_SET: u16 = 0x0001;
const ERGOHAVEN_NATIVE_KEY_ACTION_CAP_UNIVERSAL_SYMBOLS: u16 = 0x0002;
const ERGOHAVEN_NATIVE_KEY_ACTION_CAP_RUSSIAN_LETTERS: u16 = 0x0004;
const NATIVE_KEY_ACTION_STATUS_OK: u8 = 0x00;
const NATIVE_KEY_ACTION_STATUS_END: u8 = 0x01;
const NATIVE_KEY_ACTION_STATUS_UNSUPPORTED_VERSION: u8 = 0x02;
const NATIVE_KEY_ACTION_STATUS_INVALID_POSITION: u8 = 0x03;
const NATIVE_KEY_ACTION_STATUS_INVALID_PAYLOAD: u8 = 0x04;
const NATIVE_KEY_ACTION_GET_PAYLOAD_OFFSET: usize = 6;
const NATIVE_KEY_ACTION_SET_PAYLOAD_OFFSET: usize = 8;
const NATIVE_KEY_ACTION_NEXT_PAYLOAD_OFFSET: usize = 8;
const NATIVE_KEY_ACTION_MAX_PAYLOAD: usize = 32 - NATIVE_KEY_ACTION_SET_PAYLOAD_OFFSET;

const _: () = core::assert!(KeyAction::POSTCARD_MAX_SIZE <= NATIVE_KEY_ACTION_MAX_PAYLOAD);

fn process_host_data_packet(data: &[u8; 32]) -> bool {
    match data[0] {
        HOST_DATA_TIME => {
            crate::host_data::update_time(data[1], data[2]);
            true
        }
        HOST_DATA_LAYOUT => {
            crate::host_data::update_layout(data[1]);
            true
        }
        HOST_DATA_MEDIA_ARTIST => {
            crate::host_data::update_media_artist(host_data_text(data));
            true
        }
        HOST_DATA_MEDIA_TITLE => {
            crate::host_data::update_media_title(host_data_text(data));
            true
        }
        HOST_DATA_VOLUME => true,
        _ => false,
    }
}

fn host_data_text(data: &[u8; 32]) -> &str {
    let len = (data[1] as usize).min(30);
    let bytes = &data[2..2 + len];
    match core::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => core::str::from_utf8(&bytes[..err.valid_up_to()]).unwrap_or(""),
    }
}

fn battery_level_byte(status: rmk_types::battery::BatteryStatus) -> Option<u8> {
    match status {
        rmk_types::battery::BatteryStatus::Available { level: Some(level), .. } if level <= 100 => Some(level),
        _ => None,
    }
}

fn init_native_key_action_response(report: &mut ViaReport, subcommand: u8) {
    let command = report.output_data[0];
    report.input_data.fill(0);
    report.input_data[0] = command;
    report.input_data[1] = ERGOHAVEN_CUSTOM_NAMESPACE;
    report.input_data[2] = subcommand;
    report.input_data[3] = ERGOHAVEN_NATIVE_KEY_ACTION_VERSION;
}

fn native_key_position_valid(ctx: &KeyboardContext<'_>, layer: u8, row: u8, col: u8) -> bool {
    let (rows, cols, layers) = ctx.keymap_dimensions();
    (layer as usize) < layers && (row as usize) < rows && (col as usize) < cols
}

const fn native_key_action_capabilities() -> u16 {
    let capabilities = ERGOHAVEN_NATIVE_KEY_ACTION_CAP_GET_SET;
    #[cfg(feature = "universal_symbols")]
    let capabilities = capabilities
        | ERGOHAVEN_NATIVE_KEY_ACTION_CAP_UNIVERSAL_SYMBOLS
        | ERGOHAVEN_NATIVE_KEY_ACTION_CAP_RUSSIAN_LETTERS;
    capabilities
}

fn encode_native_key_action(report: &mut ViaReport, payload_offset: usize, action: KeyAction) -> bool {
    let mut encoded = [0u8; NATIVE_KEY_ACTION_MAX_PAYLOAD];
    let Ok(bytes) = postcard::to_slice(&action, &mut encoded) else {
        return false;
    };
    if payload_offset + bytes.len() > report.input_data.len() {
        return false;
    }
    report.input_data[payload_offset - 1] = bytes.len() as u8;
    report.input_data[payload_offset..payload_offset + bytes.len()].copy_from_slice(bytes);
    true
}

fn process_native_key_action_get(report: &mut ViaReport, ctx: &KeyboardContext<'_>) {
    init_native_key_action_response(report, ERGOHAVEN_CUSTOM_NATIVE_KEY_ACTION);
    if report.output_data[3] != ERGOHAVEN_NATIVE_KEY_ACTION_VERSION {
        report.input_data[4] = NATIVE_KEY_ACTION_STATUS_UNSUPPORTED_VERSION;
        return;
    }
    let (layer, row, col) = (report.output_data[4], report.output_data[5], report.output_data[6]);
    if !native_key_position_valid(ctx, layer, row, col) {
        report.input_data[4] = NATIVE_KEY_ACTION_STATUS_INVALID_POSITION;
        return;
    }
    let action = ctx.get_action(layer, row, col);
    if !encode_native_key_action(report, NATIVE_KEY_ACTION_GET_PAYLOAD_OFFSET, action) {
        report.input_data[4] = NATIVE_KEY_ACTION_STATUS_INVALID_PAYLOAD;
    }
}

async fn process_native_key_action_set(report: &mut ViaReport, ctx: &KeyboardContext<'_>) {
    init_native_key_action_response(report, ERGOHAVEN_CUSTOM_NATIVE_KEY_ACTION);
    if report.output_data[3] != ERGOHAVEN_NATIVE_KEY_ACTION_VERSION {
        report.input_data[4] = NATIVE_KEY_ACTION_STATUS_UNSUPPORTED_VERSION;
        return;
    }
    let (layer, row, col) = (report.output_data[4], report.output_data[5], report.output_data[6]);
    if !native_key_position_valid(ctx, layer, row, col) {
        report.input_data[4] = NATIVE_KEY_ACTION_STATUS_INVALID_POSITION;
        return;
    }
    let payload_len = report.output_data[7] as usize;
    if payload_len == 0 || payload_len > NATIVE_KEY_ACTION_MAX_PAYLOAD {
        report.input_data[4] = NATIVE_KEY_ACTION_STATUS_INVALID_PAYLOAD;
        return;
    }
    let payload =
        &report.output_data[NATIVE_KEY_ACTION_SET_PAYLOAD_OFFSET..NATIVE_KEY_ACTION_SET_PAYLOAD_OFFSET + payload_len];
    let Ok(action) = postcard::from_bytes::<KeyAction>(payload) else {
        report.input_data[4] = NATIVE_KEY_ACTION_STATUS_INVALID_PAYLOAD;
        return;
    };
    ctx.set_action(layer, row, col, action).await;
}

fn process_next_native_key_action_get(report: &mut ViaReport, ctx: &KeyboardContext<'_>) {
    init_native_key_action_response(report, ERGOHAVEN_CUSTOM_NEXT_NATIVE_KEY_ACTION);
    if report.output_data[3] != ERGOHAVEN_NATIVE_KEY_ACTION_VERSION {
        report.input_data[4] = NATIVE_KEY_ACTION_STATUS_UNSUPPORTED_VERSION;
        return;
    }
    let start = LittleEndian::read_u16(&report.output_data[4..6]) as usize;
    let (rows, cols, layers) = ctx.keymap_dimensions();
    let total = rows.saturating_mul(cols).saturating_mul(layers);
    for flat_index in start..total.min(u16::MAX as usize) {
        let action = ctx.get_action_flat(flat_index);
        if action != KeyAction::No && to_via_keycode(action) == 0 {
            LittleEndian::write_u16(&mut report.input_data[5..7], flat_index as u16);
            if !encode_native_key_action(report, NATIVE_KEY_ACTION_NEXT_PAYLOAD_OFFSET, action) {
                report.input_data[4] = NATIVE_KEY_ACTION_STATUS_INVALID_PAYLOAD;
            }
            return;
        }
    }
    report.input_data[4] = NATIVE_KEY_ACTION_STATUS_END;
    LittleEndian::write_u16(&mut report.input_data[5..7], u16::MAX);
}

fn battery_halves_for_split(
    central: BatteryStatus,
    peripheral_0: BatteryStatus,
    peripheral_1: BatteryStatus,
    peripheral_count: usize,
    central_is_left: bool,
) -> (BatteryStatus, BatteryStatus) {
    if peripheral_count == 1 {
        if central_is_left {
            (central, peripheral_0)
        } else {
            (peripheral_0, central)
        }
    } else {
        (peripheral_0, peripheral_1)
    }
}

pub struct VialService<'a> {
    ctx: &'a KeyboardContext<'a>,
    vial_config: VialConfig<'static>,
    #[cfg(feature = "vial_lock")]
    locker: vial_lock::VialLock<'a>,
}

impl<'a> VialService<'a> {
    pub fn new(ctx: &'a KeyboardContext<'a>, config: &RmkConfig<'static>) -> Self {
        Self {
            ctx,
            vial_config: config.vial_config,
            #[cfg(feature = "vial_lock")]
            locker: vial_lock::VialLock::new(
                config.vial_config.unlock_keys,
                ctx.keymap,
                config.vial_config.vial_insecure,
            ),
        }
    }

    async fn process_via_packet(&mut self, report: &mut ViaReport) {
        let command_id = report.output_data[0];

        // Caller pre-fills `input_data` from `output_data`, so individual arms
        // only need to overwrite the bytes they actually change.
        match command_id.into() {
            ViaCommand::GetProtocolVersion => {
                BigEndian::write_u16(&mut report.input_data[1..3], VIA_PROTOCOL_VERSION);
            }
            ViaCommand::GetKeyboardValue => {
                // Check the second u8
                match report.output_data[1].try_into() {
                    Ok(v) => match v {
                        ViaKeyboardInfo::Uptime => {
                            let value = Instant::now().as_millis() as u32;
                            BigEndian::write_u32(&mut report.input_data[2..6], value);
                        }
                        ViaKeyboardInfo::LayoutOptions => {
                            let layout_option = self.ctx.layout_options().await;
                            BigEndian::write_u32(&mut report.input_data[2..6], layout_option);
                        }
                        #[cfg(not(feature = "vial_lock"))]
                        ViaKeyboardInfo::SwitchMatrixState => {
                            error!("It is not secure to use matrix tester without vial lock");
                        }
                        #[cfg(feature = "vial_lock")]
                        ViaKeyboardInfo::SwitchMatrixState if self.locker.is_unlocked() => {
                            self.ctx.read_matrix_state(&mut report.input_data[2..]);
                        }
                        ViaKeyboardInfo::FirmwareVersion => {
                            BigEndian::write_u32(&mut report.input_data[2..6], self.vial_config.firmware_version);
                        }
                        _ => (),
                    },
                    Err(e) => error!("Invalid subcommand: {} of GetKeyboardValue", e),
                }
            }
            ViaCommand::SetKeyboardValue => {
                // Check the second u8
                match report.output_data[1].try_into() {
                    Ok(v) => match v {
                        ViaKeyboardInfo::LayoutOptions => {
                            let layout_option = BigEndian::read_u32(&report.output_data[2..6]);
                            self.ctx.set_layout_options(layout_option).await;
                        }
                        ViaKeyboardInfo::DeviceIndication => {
                            let _device_indication = report.output_data[2];
                            warn!("SetKeyboardValue - DeviceIndication")
                        }
                        _ => (),
                    },
                    Err(e) => error!("Invalid subcommand: {} of GetKeyboardValue", e),
                }
            }
            ViaCommand::DynamicKeymapGetKeyCode => {
                let layer = report.output_data[1];
                let row = report.output_data[2];
                let col = report.output_data[3];
                let action = self.ctx.get_action(layer, row, col);
                let keycode = to_via_keycode(action);
                info!("Getting keycode: {:02X} at ({},{}), layer {}", keycode, row, col, layer);
                BigEndian::write_u16(&mut report.input_data[4..6], keycode);
            }
            ViaCommand::DynamicKeymapSetKeyCode => {
                let layer = report.output_data[1];
                let row = report.output_data[2];
                let col = report.output_data[3];
                let keycode = BigEndian::read_u16(&report.output_data[4..6]);
                let action = from_via_keycode(keycode);
                info!(
                    "Setting keycode: 0x{:02X} at ({},{}), layer {} as {:?}",
                    keycode, row, col, layer, action
                );
                self.ctx.set_action(layer, row, col, action).await;
            }
            ViaCommand::DynamicKeymapReset => {
                warn!("Dynamic keymap reset -- not supported")
            }
            ViaCommand::CustomSetValue => {
                if report.output_data[1] == ERGOHAVEN_CUSTOM_NAMESPACE
                    && report.output_data[2] == ERGOHAVEN_CUSTOM_NATIVE_KEY_ACTION
                {
                    process_native_key_action_set(report, self.ctx).await;
                } else {
                    // backlight/rgblight/rgb matrix/led matrix/audio settings here
                    warn!("Custom set value -- not supported")
                }
            }
            ViaCommand::CustomGetValue => {
                if report.output_data[1] == ERGOHAVEN_CUSTOM_NAMESPACE
                    && report.output_data[2] == ERGOHAVEN_CUSTOM_BATTERY_HALVES
                {
                    #[cfg(all(feature = "split", feature = "_ble"))]
                    crate::event::publish_event(crate::event::PeripheralBatteryRefreshEvent);

                    report.input_data[3] = ERGOHAVEN_BATTERY_HALVES_VERSION;
                    report.input_data[4] = 0;
                    report.input_data[5] = 0xFF;
                    report.input_data[6] = 0xFF;

                    #[cfg(all(feature = "split", feature = "_ble"))]
                    {
                        let (left, right) = battery_halves_for_split(
                            self.ctx.battery_status(),
                            self.ctx.peripheral_battery_status(0),
                            self.ctx.peripheral_battery_status(1),
                            crate::SPLIT_PERIPHERALS_NUM,
                            crate::SPLIT_CENTRAL_IS_LEFT,
                        );
                        if let Some(level) = battery_level_byte(left) {
                            report.input_data[4] |= 0x01;
                            report.input_data[5] = level;
                        }
                        if let Some(level) = battery_level_byte(right) {
                            report.input_data[4] |= 0x02;
                            report.input_data[6] = level;
                        }
                    }
                } else if report.output_data[1] == ERGOHAVEN_CUSTOM_NAMESPACE
                    && report.output_data[2] == ERGOHAVEN_CUSTOM_NATIVE_KEY_ACTION_CAPS
                {
                    init_native_key_action_response(report, ERGOHAVEN_CUSTOM_NATIVE_KEY_ACTION_CAPS);
                    LittleEndian::write_u16(&mut report.input_data[4..6], native_key_action_capabilities());
                } else if report.output_data[1] == ERGOHAVEN_CUSTOM_NAMESPACE
                    && report.output_data[2] == ERGOHAVEN_CUSTOM_NATIVE_KEY_ACTION
                {
                    process_native_key_action_get(report, self.ctx);
                } else if report.output_data[1] == ERGOHAVEN_CUSTOM_NAMESPACE
                    && report.output_data[2] == ERGOHAVEN_CUSTOM_NEXT_NATIVE_KEY_ACTION
                {
                    process_next_native_key_action_get(report, self.ctx);
                } else {
                    // backlight/rgblight/rgb matrix/led matrix/audio settings here
                    warn!("Custom get value -- not supported")
                }
            }
            ViaCommand::CustomSave => {
                // backlight/rgblight/rgb matrix/led matrix/audio settings here
                warn!("Custom get value -- not supported")
            }
            ViaCommand::EepromReset => {
                warn!("Resetting storage..");
                self.ctx.reset_storage().await;
                // TODO: Reboot after a eeprom reset?
            }
            ViaCommand::BootloaderJump => {
                warn!("Bootloader jumping");
                boot::jump_to_bootloader();
            }
            ViaCommand::DynamicKeymapMacroGetCount => {
                report.input_data[1] = 32;
                warn!("Macro get count -- to be implemented")
            }
            ViaCommand::DynamicKeymapMacroGetBufferSize => {
                report.input_data[1] = (MACRO_SPACE_SIZE as u16 >> 8) as u8;
                report.input_data[2] = (MACRO_SPACE_SIZE & 0xFF) as u8;
            }
            ViaCommand::DynamicKeymapMacroGetBuffer => {
                let offset = BigEndian::read_u16(&report.output_data[1..3]) as usize;
                let size = report.output_data[3] as usize;
                if size <= 28 {
                    self.ctx.read_macro_buffer(offset, &mut report.input_data[4..4 + size]);
                    debug!("Get macro buffer: offset: {}, data: {:?}", offset, report.input_data);
                } else {
                    report.input_data[0] = 0xFF;
                }
            }
            ViaCommand::DynamicKeymapMacroSetBuffer => {
                // Every write writes all buffer space of the macro(if it's not empty)
                let offset = BigEndian::read_u16(&report.output_data[1..3]);
                // Current sequence size, <= 28
                let size = report.output_data[3];
                // `output_data` is 32 bytes, so the payload slice output_data[4..4 + size]
                // is only valid for size <= 28. Reject oversized writes instead of
                // panicking, mirroring the DynamicKeymapMacroGetBuffer handler above.
                if size <= 28 {
                    // End of current sequence in the macro cache
                    // The first sequence, reset the macro cache
                    if offset == 0 {
                        self.ctx.reset_macro_buffer();
                    }

                    // Update macro cache + flush full buffer to storage
                    info!("Setting macro buffer, offset: {}, size: {}", offset, size);
                    self.ctx
                        .write_macro_buffer(offset as usize, &report.output_data[4..4 + size as usize])
                        .await;
                } else {
                    report.input_data[0] = 0xFF;
                }
            }
            ViaCommand::DynamicKeymapMacroReset => {
                warn!("Macro reset -- to be implemented")
            }
            ViaCommand::DynamicKeymapGetLayerCount => {
                report.input_data[1] = self.ctx.keymap_dimensions().2 as u8;
            }
            ViaCommand::DynamicKeymapGetBuffer => {
                let offset = BigEndian::read_u16(&report.output_data[1..3]);
                // size <= 28
                let size = report.output_data[3];
                debug!("Getting keymap buffer, offset: {}, size: {}", offset, size);
                let mut idx = 4;
                let start = (offset / 2) as usize;
                let count = (size / 2) as usize;
                for i in 0..count {
                    let a = self.ctx.get_action_flat(start + i);
                    let kc = to_via_keycode(a);
                    BigEndian::write_u16(&mut report.input_data[idx..idx + 2], kc);
                    idx += 2;
                }
            }
            ViaCommand::DynamicKeymapSetBuffer => {
                debug!("Dynamic keymap set buffer");
                let offset = BigEndian::read_u16(&report.output_data[1..3]);
                // size <= 28
                let size = report.output_data[3];
                let mut idx = 4;
                let (rows, cols, _) = self.ctx.keymap_dimensions();
                for i in 0..(size as usize) {
                    let via_keycode = LittleEndian::read_u16(&report.output_data[idx..idx + 2]);
                    let action = from_via_keycode(via_keycode);
                    let flat_index = offset as usize + i;
                    self.ctx.try_set_action_flat(flat_index, action, rows, cols);
                    idx += 2;
                }
            }
            ViaCommand::DynamicKeymapGetEncoder => {
                warn!("Keymap get encoder -- not supported");
            }
            ViaCommand::DynamicKeymapSetEncoder => {
                warn!("Keymap set encoder -- not supported");
            }
            ViaCommand::Vial => {
                process_vial(
                    report,
                    &self.vial_config,
                    #[cfg(feature = "vial_lock")]
                    &mut self.locker,
                    self.ctx,
                )
                .await
            }
            ViaCommand::Unhandled => {
                info!("Unknown cmd: {:?}", report.output_data);
                report.input_data[0] = ViaCommand::Unhandled as u8
            }
        }
    }
}

impl Runnable for VialService<'_> {
    async fn run(&mut self) -> ! {
        loop {
            let (transport, output_data) = HOST_REQUEST_CHANNEL.receive().await;
            if process_host_data_packet(&output_data) {
                continue;
            }
            let mut report = ViaReport {
                input_data: output_data,
                output_data,
            };
            self.process_via_packet(&mut report).await;
            try_send_host_reply(transport, report.input_data);
        }
    }
}

#[cfg(test)]
mod tests {
    use embassy_futures::block_on;
    use rmk_types::action::{Action, KeyAction};
    use rmk_types::battery::ChargeState;
    use rmk_types::keycode::{HidKeyCode, KeyCode};
    use rmk_types::modifier::ModifierCombination;

    use super::*;
    use crate::config::{BehaviorConfig, PositionalConfig};
    use crate::keymap::{KeyMap, KeymapData};

    /// Build a minimal 1x1x1 keymap + `VialService` and run `f` against it.
    fn with_service<R>(f: impl FnOnce(&mut VialService) -> R) -> R {
        let mut data = KeymapData::new([[[KeyAction::No]]]);
        let mut behavior = BehaviorConfig::default();
        let positional = PositionalConfig::<1, 1>::default();
        let keymap = block_on(KeyMap::new(&mut data, &mut behavior, &positional));
        let ctx = KeyboardContext::new(&keymap);
        let config = RmkConfig::default();
        let mut service = VialService::new(&ctx, &config);
        f(&mut service)
    }

    /// A `DynamicKeymapMacroSetBuffer` (0x0F) report with `offset = 0` and the
    /// given payload `size` byte. The caller mirrors `Runnable::run` by seeding
    /// `input_data` with a copy of `output_data`.
    fn macro_set_buffer_report(size: u8) -> ViaReport {
        let mut output_data = [0u8; 32];
        output_data[0] = 0x0F; // DynamicKeymapMacroSetBuffer
        output_data[3] = size;
        ViaReport {
            input_data: output_data,
            output_data,
        }
    }

    fn custom_report(command: ViaCommand, subcommand: u8) -> ViaReport {
        let mut output_data = [0u8; 32];
        output_data[0] = command as u8;
        output_data[1] = ERGOHAVEN_CUSTOM_NAMESPACE;
        output_data[2] = subcommand;
        ViaReport {
            input_data: output_data,
            output_data,
        }
    }

    fn rich_mod_tap() -> KeyAction {
        KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc0, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::LCTRL),
            Default::default(),
        )
    }

    #[test]
    fn k04_micro_factory_mod_actions_keep_their_lossless_transport_path() {
        let standard_left_shift = KeyAction::TapHold(
            Action::Key(KeyCode::Hid(HidKeyCode::Minus)),
            Action::Modifier(ModifierCombination::LSHIFT),
            Default::default(),
        );
        let rich_left_ctrl = rich_mod_tap();
        let shifted_equal = KeyAction::Single(Action::KeyWithModifier(HidKeyCode::Equal, ModifierCombination::LSHIFT));
        let shifted_five = KeyAction::Single(Action::KeyWithModifier(HidKeyCode::Kc5, ModifierCombination::LSHIFT));
        let rich_right_ctrl = KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::LeftBracket, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::RCTRL),
            Default::default(),
        );
        let standard_right_shift = KeyAction::TapHold(
            Action::Key(KeyCode::Hid(HidKeyCode::Semicolon)),
            Action::Modifier(ModifierCombination::RSHIFT),
            Default::default(),
        );

        assert_eq!(to_via_keycode(standard_left_shift), 0x222D);
        assert_eq!(to_via_keycode(rich_left_ctrl), 0);
        assert_eq!(to_via_keycode(shifted_equal), 0x022E);
        assert_eq!(to_via_keycode(shifted_five), 0x0222);
        assert_eq!(to_via_keycode(rich_right_ctrl), 0);
        assert_eq!(to_via_keycode(standard_right_shift), 0x3233);
    }

    #[test]
    fn reports_the_configured_runtime_firmware_version() {
        let mut data = KeymapData::new([[[KeyAction::No]]]);
        let mut behavior = BehaviorConfig::default();
        let positional = PositionalConfig::<1, 1>::default();
        let keymap = block_on(KeyMap::new(&mut data, &mut behavior, &positional));
        let ctx = KeyboardContext::new(&keymap);
        let config = RmkConfig {
            vial_config: VialConfig {
                firmware_version: 0x0000_0103,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut service = VialService::new(&ctx, &config);
        let mut output_data = [0u8; 32];
        output_data[0] = ViaCommand::GetKeyboardValue as u8;
        output_data[1] = ViaKeyboardInfo::FirmwareVersion as u8;
        let mut report = ViaReport {
            input_data: output_data,
            output_data,
        };

        block_on(service.process_via_packet(&mut report));

        assert_eq!(&report.input_data[2..6], &[0x00, 0x00, 0x01, 0x03]);
    }

    // `output_data` is [u8; 32], so the handler slices `output_data[4..4 + size]`.
    // size == 28 is the largest payload that fits (writes output_data[4..32]).
    #[test]
    fn macro_set_buffer_max_size_ok() {
        with_service(|service| {
            let mut report = macro_set_buffer_report(28);
            block_on(service.process_via_packet(&mut report));
        });
    }

    // size == 29 slices output_data[4..33], which is out of bounds. The sibling
    // DynamicKeymapMacroGetBuffer handler already rejects size > 28 with 0xFF;
    // SetBuffer must do the same instead of panicking.
    #[test]
    fn macro_set_buffer_oversize_rejected() {
        with_service(|service| {
            let mut report = macro_set_buffer_report(29);
            block_on(service.process_via_packet(&mut report));
            assert_eq!(report.input_data[0], 0xFF);
        });
    }

    #[test]
    fn native_key_action_capability_is_versioned() {
        with_service(|service| {
            let mut report = custom_report(ViaCommand::CustomGetValue, ERGOHAVEN_CUSTOM_NATIVE_KEY_ACTION_CAPS);
            block_on(service.process_via_packet(&mut report));
            assert_eq!(report.input_data[3], ERGOHAVEN_NATIVE_KEY_ACTION_VERSION);
            assert_eq!(
                LittleEndian::read_u16(&report.input_data[4..6]),
                native_key_action_capabilities()
            );
        });
    }

    #[test]
    fn native_key_action_set_and_get_round_trip() {
        with_service(|service| {
            let action = rich_mod_tap();
            let mut encoded = [0u8; NATIVE_KEY_ACTION_MAX_PAYLOAD];
            let payload = postcard::to_slice(&action, &mut encoded).unwrap();
            let mut set_report = custom_report(ViaCommand::CustomSetValue, ERGOHAVEN_CUSTOM_NATIVE_KEY_ACTION);
            set_report.output_data[3] = ERGOHAVEN_NATIVE_KEY_ACTION_VERSION;
            set_report.output_data[7] = payload.len() as u8;
            set_report.output_data
                [NATIVE_KEY_ACTION_SET_PAYLOAD_OFFSET..NATIVE_KEY_ACTION_SET_PAYLOAD_OFFSET + payload.len()]
                .copy_from_slice(payload);
            block_on(service.process_via_packet(&mut set_report));
            assert_eq!(set_report.input_data[4], NATIVE_KEY_ACTION_STATUS_OK);

            let mut get_report = custom_report(ViaCommand::CustomGetValue, ERGOHAVEN_CUSTOM_NATIVE_KEY_ACTION);
            get_report.output_data[3] = ERGOHAVEN_NATIVE_KEY_ACTION_VERSION;
            block_on(service.process_via_packet(&mut get_report));
            assert_eq!(get_report.input_data[4], NATIVE_KEY_ACTION_STATUS_OK);
            let len = get_report.input_data[5] as usize;
            let decoded: KeyAction = postcard::from_bytes(
                &get_report.input_data
                    [NATIVE_KEY_ACTION_GET_PAYLOAD_OFFSET..NATIVE_KEY_ACTION_GET_PAYLOAD_OFFSET + len],
            )
            .unwrap();
            assert_eq!(decoded, action);
        });
    }

    #[test]
    fn native_key_action_scan_returns_only_vial_lossy_actions() {
        with_service(|service| {
            let action = rich_mod_tap();
            let mut encoded = [0u8; NATIVE_KEY_ACTION_MAX_PAYLOAD];
            let payload = postcard::to_slice(&action, &mut encoded).unwrap();
            let mut set_report = custom_report(ViaCommand::CustomSetValue, ERGOHAVEN_CUSTOM_NATIVE_KEY_ACTION);
            set_report.output_data[3] = ERGOHAVEN_NATIVE_KEY_ACTION_VERSION;
            set_report.output_data[7] = payload.len() as u8;
            set_report.output_data
                [NATIVE_KEY_ACTION_SET_PAYLOAD_OFFSET..NATIVE_KEY_ACTION_SET_PAYLOAD_OFFSET + payload.len()]
                .copy_from_slice(payload);
            block_on(service.process_via_packet(&mut set_report));

            let mut scan_report = custom_report(ViaCommand::CustomGetValue, ERGOHAVEN_CUSTOM_NEXT_NATIVE_KEY_ACTION);
            scan_report.output_data[3] = ERGOHAVEN_NATIVE_KEY_ACTION_VERSION;
            block_on(service.process_via_packet(&mut scan_report));
            assert_eq!(scan_report.input_data[4], NATIVE_KEY_ACTION_STATUS_OK);
            assert_eq!(LittleEndian::read_u16(&scan_report.input_data[5..7]), 0);
            let len = scan_report.input_data[7] as usize;
            let decoded: KeyAction = postcard::from_bytes(
                &scan_report.input_data
                    [NATIVE_KEY_ACTION_NEXT_PAYLOAD_OFFSET..NATIVE_KEY_ACTION_NEXT_PAYLOAD_OFFSET + len],
            )
            .unwrap();
            assert_eq!(decoded, action);

            LittleEndian::write_u16(&mut scan_report.output_data[4..6], 1);
            block_on(service.process_via_packet(&mut scan_report));
            assert_eq!(scan_report.input_data[4], NATIVE_KEY_ACTION_STATUS_END);
        });
    }

    fn battery(level: u8) -> BatteryStatus {
        BatteryStatus::Available {
            charge_state: ChargeState::Unknown,
            level: Some(level),
        }
    }

    #[test]
    fn no_qube_split_uses_central_and_first_peripheral_batteries() {
        assert_eq!(
            battery_halves_for_split(battery(80), battery(55), BatteryStatus::Unavailable, 1, true,),
            (battery(80), battery(55))
        );
    }

    #[test]
    fn right_central_split_reports_physical_battery_order() {
        assert_eq!(
            battery_halves_for_split(battery(80), battery(55), BatteryStatus::Unavailable, 1, false,),
            (battery(55), battery(80))
        );
    }

    #[test]
    fn qube_split_uses_both_peripheral_batteries() {
        assert_eq!(
            battery_halves_for_split(battery(100), battery(80), battery(55), 2, true),
            (battery(80), battery(55))
        );
    }
}
