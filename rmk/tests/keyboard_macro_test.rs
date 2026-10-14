pub mod common;

mod macro_test {
    use heapless::Vec;
    use rmk::config::{BehaviorConfig, PositionalConfig};
    use rmk::keyboard::Keyboard;
    use rmk::keyboard_macros::{MacroOperation, define_macro_sequences, to_macro_sequence};
    use rmk::types::action::{Action, KeyAction};
    use rmk_types::keycode::HidKeyCode;

    use crate::common::{KC_LSHIFT, wrap_keymap};
    use crate::{kc_to_u8, key_sequence_test};

    fn create_simple_macro_keyboard(behavior_config: BehaviorConfig) -> Keyboard<'static> {
        let keymap = [[[
            KeyAction::Single(Action::TriggerMacro(0)),
            KeyAction::Single(Action::TriggerMacro(1)),
        ]]];
        let behavior_config: &'static mut BehaviorConfig = Box::leak(Box::new(behavior_config));
        let per_key_config: &'static PositionalConfig<1, 2> = Box::leak(Box::new(PositionalConfig::default()));
        Keyboard::new(wrap_keymap(keymap, per_key_config, behavior_config))
    }

    // Exercise the real decoder, keyboard runner, and report queue. Observe every
    // report variant (the ordinary key-sequence helper ignores non-keyboard HID).
    fn capture_macro_reports(bytes: &[u8]) -> std::vec::Vec<(u64, rmk::hid::Report)> {
        use embassy_futures::select::{Either, select};
        use embassy_time::{Duration, Instant, Timer, with_deadline};
        use rmk::channel::USB_REPORT_CHANNEL;
        use rmk::core_traits::Runnable;
        use rmk::event::{AsyncEventPublisher, AsyncPublishableEvent, KeyboardEvent};
        use rmk::state::set_usb_state;
        use rmk::types::connection::UsbState;

        let mut config = BehaviorConfig::default();
        config.keyboard_macros.macro_sequences[..bytes.len()].copy_from_slice(bytes);
        let mut keyboard = create_simple_macro_keyboard(config);
        let sender = KeyboardEvent::publisher_async();
        sender.clear();
        USB_REPORT_CHANNEL.clear();
        set_usb_state(UsbState::Configured);
        crate::common::test_block_on::test_block_on(async {
            let observe = async {
                let send = async {
                    sender.publish_async(KeyboardEvent::key(0, 0, true)).await;
                    Timer::after_millis(10).await;
                    sender.publish_async(KeyboardEvent::key(0, 0, false)).await;
                };
                let collect = async {
                    let start = Instant::now();
                    let end = start + Duration::from_millis(600);
                    let mut reports = std::vec::Vec::new();
                    while let Ok(queued) = with_deadline(end, USB_REPORT_CHANNEL.receive()).await {
                        reports.push((start.elapsed().as_millis(), queued.into_report()));
                    }
                    reports
                };
                let (_, reports) = futures::join!(send, collect);
                reports
            };
            match select(keyboard.run(), observe).await {
                Either::Second(reports) => reports,
                Either::First(_) => unreachable!(),
            }
        })
    }

    #[test]
    fn test_macro_short_mouse_button_routing_and_polarity() {
        use rmk::hid::Report;
        // Raw, canonical short opcodes: Tap, Down, Up Mouse1 in M0.
        let reports = capture_macro_reports(&[1, 1, 0xD1, 1, 2, 0xD1, 1, 3, 0xD1, 0]);
        assert_eq!(reports.len(), 4, "reports: {reports:?}");
        for ((_, report), buttons) in reports.iter().zip([1, 0, 1, 0]) {
            match report {
                Report::MouseReport(r) => assert_eq!((r.buttons, r.x, r.y, r.wheel, r.pan), (buttons, 0, 0, 0, 0)),
                other => panic!("short mouse operation must emit mouse HID, got {other:?}"),
            }
        }
    }

    fn assert_held_mouse_repeats(key: u8, wheel: bool, interval: u64, repeat_count: usize) {
        use rmk::hid::Report;
        // Down, Delay(350 ms), Up. Delay is Vial's nonzero base-255 encoding.
        let reports = capture_macro_reports(&[1, 2, key, 1, 4, 96, 2, 1, 3, key, 0]);
        assert_eq!(
            reports.len(),
            repeat_count + 2,
            "initial, repeats, release: {reports:?}"
        );
        for (i, (_, report)) in reports.iter().enumerate() {
            let Report::MouseReport(r) = report else {
                panic!("expected mouse HID, got {report:?}")
            };
            assert_eq!(r.buttons, 0);
            assert_eq!((r.y, r.pan), (0, 0));
            if i == reports.len() - 1 {
                assert_eq!((r.x, r.wheel), (0, 0), "release stops movement");
            } else if wheel {
                assert_eq!(r.x, 0);
                assert!(r.wheel < 0);
            } else {
                assert_eq!(r.wheel, 0);
                assert!(r.x > 0);
            }
        }
        let first_repeat_delay = reports[1].0 - reports[0].0;
        assert!(
            (100..=101).contains(&first_repeat_delay),
            "initial deadline: {reports:?}"
        );
        for pair in reports[1..=repeat_count].windows(2) {
            let elapsed = pair[1].0 - pair[0].0;
            assert!(
                (interval..=interval + 1).contains(&elapsed),
                "repeat interval: {reports:?}"
            );
        }
        assert!(
            (350..=355).contains(&(reports.last().unwrap().0 - reports[0].0)),
            "delay must not restart: {reports:?}"
        );
    }

    #[test]
    fn test_macro_held_movement_repeats_during_delay() {
        assert_held_mouse_repeats(0xD0, false, 20, 13);
    }

    #[test]
    fn test_macro_held_wheel_repeats_during_delay() {
        assert_held_mouse_repeats(0xD9, true, 80, 4);
    }

    #[test]
    fn test_macro_short_consumer_routing_and_polarity() {
        use rmk::hid::Report;
        let reports = capture_macro_reports(&[1, 1, 0xA9, 1, 2, 0xA9, 1, 3, 0xA9, 0]);
        assert_eq!(reports.len(), 4);
        for ((_, report), usage) in reports.iter().zip([0xE9, 0, 0xE9, 0]) {
            match report {
                Report::MediaKeyboardReport(r) => assert_eq!({ r.usage_id }, usage),
                other => panic!("expected consumer HID, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_macro_short_system_routing_and_polarity() {
        use rmk::hid::Report;
        let reports = capture_macro_reports(&[1, 1, 0xA6, 1, 2, 0xA6, 1, 3, 0xA6, 0]);
        assert_eq!(reports.len(), 4);
        for ((_, report), usage) in reports.iter().zip([0x82, 0, 0x82, 0]) {
            match report {
                Report::SystemControlReport(r) => assert_eq!(r.usage_id, usage),
                other => panic!("expected system HID, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_macro_short_all_mouse_buttons() {
        use rmk::hid::Report;
        let mut bytes = std::vec::Vec::new();
        for key in 0xD1..=0xD8 {
            bytes.extend_from_slice(&[1, 1, key]);
        }
        bytes.push(0);
        let reports = capture_macro_reports(&bytes);
        assert_eq!(reports.len(), 16);
        for (index, pair) in reports.chunks_exact(2).enumerate() {
            for ((_, report), buttons) in pair.iter().zip([1 << index, 0]) {
                match report {
                    Report::MouseReport(r) => assert_eq!((r.buttons, r.x, r.y, r.wheel, r.pan), (buttons, 0, 0, 0, 0)),
                    other => panic!("expected mouse HID, got {other:?}"),
                }
            }
        }
    }

    #[test]
    fn test_macro_short_all_mouse_directions() {
        use rmk::hid::Report;
        let bytes = [
            1, 1, 0xCD, 1, 1, 0xCE, 1, 1, 0xCF, 1, 1, 0xD0, 1, 1, 0xD9, 1, 1, 0xDA, 1, 1, 0xDB, 1, 1, 0xDC, 0,
        ];
        let reports = capture_macro_reports(&bytes);
        assert_eq!(reports.len(), 16, "reports: {reports:?}");
        let axes = [
            (0, -5, 0, 0),
            (0, 5, 0, 0),
            (-5, 0, 0, 0),
            (5, 0, 0, 0),
            (0, 0, -1, 0),
            (0, 0, 1, 0),
            (0, 0, 0, -1),
            (0, 0, 0, 1),
        ];
        for (pair, expected) in reports.chunks_exact(2).zip(axes) {
            for ((_, report), axes) in pair.iter().zip([expected, (0, 0, 0, 0)]) {
                match report {
                    Report::MouseReport(r) => {
                        assert_eq!(r.buttons, 0);
                        assert_eq!((r.x, r.y, r.wheel, r.pan), axes);
                    }
                    other => panic!("expected mouse HID, got {other:?}"),
                }
            }
        }
    }

    #[test]
    fn test_macro_short_mouse_acceleration_uses_shared_state() {
        use rmk::hid::Report;
        // Down Accel0, Tap Right, Up Accel0, Tap Right. Accel emits no HID itself.
        let reports = capture_macro_reports(&[1, 2, 0xDD, 1, 1, 0xD0, 1, 3, 0xDD, 1, 1, 0xD0, 0]);
        assert_eq!(reports.len(), 4, "reports: {reports:?}");
        for ((_, report), x) in reports.iter().zip([2, 0, 5, 0]) {
            match report {
                Report::MouseReport(r) => assert_eq!((r.buttons, r.x, r.y, r.wheel, r.pan), (0, x, 0, 0, 0)),
                other => panic!("expected mouse HID, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_macro_basic_down_remains_held_during_delay() {
        use rmk::hid::Report;
        let reports = capture_macro_reports(&[1, 2, 4, 1, 4, 96, 2, 1, 3, 4, 0]);
        assert_eq!(reports.len(), 2);
        for ((_, report), keys) in reports.iter().zip([[4, 0, 0, 0, 0, 0], [0; 6]]) {
            match report {
                Report::KeyboardReport(r) => {
                    assert_eq!(r.modifier, 0);
                    assert_eq!(r.keycodes, keys);
                }
                other => panic!("expected keyboard HID, got {other:?}"),
            }
        }
        assert!((350..=355).contains(&(reports[1].0 - reports[0].0)));
    }

    #[test]
    fn test_macro_key_a_press_release() {
        let macro_sequences = &[Vec::from_slice(&[
            MacroOperation::Press(HidKeyCode::A),
            MacroOperation::Release(HidKeyCode::A),
        ])
        .expect("too many elements")];

        let macro_data = define_macro_sequences(macro_sequences);
        let mut config = BehaviorConfig::default();
        config.keyboard_macros.macro_sequences = macro_data;

        let keyboard = create_simple_macro_keyboard(config);

        key_sequence_test!(
            keyboard: keyboard,
            sequence: [
                [0, 0, true, 0],   // press Macro0
                [0, 0, false, 100], // release Macro0
            ],
            expected_reports: [
                [0, [kc_to_u8!(A), 0, 0, 0, 0, 0]], // press A
                [0, [0, 0, 0, 0, 0, 0]],            // release A
            ]
        );
    }

    #[test]
    fn test_macro_text() {
        let macro_sequences = &[to_macro_sequence("AbCd123456")];

        let macro_data = define_macro_sequences(macro_sequences);
        let mut config = BehaviorConfig::default();
        config.keyboard_macros.macro_sequences = macro_data;

        let keyboard = create_simple_macro_keyboard(config);

        key_sequence_test!(
            keyboard: keyboard,
            sequence: [
                [0, 0, true, 0],   // press Macro0
                [0, 0, false, 100], // release Macro0
            ],
            expected_reports: [
                [KC_LSHIFT, [0, 0, 0, 0, 0, 0]],            // press shift
                [KC_LSHIFT, [kc_to_u8!(A), 0, 0, 0, 0, 0]], // press A + shift
                [KC_LSHIFT, [0, 0, 0, 0, 0, 0]],            // release A
                [0, [0, 0, 0, 0, 0, 0]],            // release shift
                [0, [kc_to_u8!(B), 0, 0, 0, 0, 0]], // press B
                [0, [0, 0, 0, 0, 0, 0]],            // release B
                [KC_LSHIFT, [0, 0, 0, 0, 0, 0]],            // press shift
                [KC_LSHIFT, [kc_to_u8!(C), 0, 0, 0, 0, 0]], // press C + shift
                [KC_LSHIFT, [0, 0, 0, 0, 0, 0]],            // release C
                [0, [0, 0, 0, 0, 0, 0]],            // release shift
                [0, [kc_to_u8!(D), 0, 0, 0, 0, 0]], // press D
                [0, [0, 0, 0, 0, 0, 0]],            // release D
                [0, [kc_to_u8!(Kc1), 0, 0, 0, 0, 0]], // press 1
                [0, [0, 0, 0, 0, 0, 0]],            // release 1
                [0, [kc_to_u8!(Kc2), 0, 0, 0, 0, 0]], // press 2
                [0, [0, 0, 0, 0, 0, 0]],            // release 2
                [0, [kc_to_u8!(Kc3), 0, 0, 0, 0, 0]], // press 3
                [0, [0, 0, 0, 0, 0, 0]],            // release 3
                [0, [kc_to_u8!(Kc4), 0, 0, 0, 0, 0]], // press 4
                [0, [0, 0, 0, 0, 0, 0]],            // release 4
                [0, [kc_to_u8!(Kc5), 0, 0, 0, 0, 0]], // press 5
                [0, [0, 0, 0, 0, 0, 0]],            // release 5
                [0, [kc_to_u8!(Kc6), 0, 0, 0, 0, 0]], // press 6
                [0, [0, 0, 0, 0, 0, 0]],            // release 6
            ]
        );
    }

    #[test]
    fn test_macro_tap_key_a() {
        let macro_sequences = &[Vec::from_slice(&[MacroOperation::Tap(HidKeyCode::A)]).expect("too many elements")];

        let macro_data = define_macro_sequences(macro_sequences);
        let mut config = BehaviorConfig::default();
        config.keyboard_macros.macro_sequences = macro_data;

        let keyboard = create_simple_macro_keyboard(config);

        key_sequence_test!(
            keyboard: keyboard,
            sequence: [
                [0, 0, true, 0],   // press Macro0
                [0, 0, false, 100], // release Macro0
            ],
            expected_reports: [
                [0, [kc_to_u8!(A), 0, 0, 0, 0, 0]], // press A
                [0, [0, 0, 0, 0, 0, 0]],            // release A
            ]
        );
    }

    #[test]
    fn test_macro_multiple_operations() {
        let macro_sequences = &[Vec::from_slice(&[
            MacroOperation::Press(HidKeyCode::LShift),
            MacroOperation::Tap(HidKeyCode::A),
            MacroOperation::Release(HidKeyCode::LShift),
            MacroOperation::Tap(HidKeyCode::B),
        ])
        .expect("too many elements")];

        let macro_data = define_macro_sequences(macro_sequences);
        let mut config = BehaviorConfig::default();
        config.keyboard_macros.macro_sequences = macro_data;

        let keyboard = create_simple_macro_keyboard(config);

        key_sequence_test!(
            keyboard: keyboard,
            sequence: [
                [0, 0, true, 0],   // press macro0
                [0, 0, false, 100], // release macro0
            ],
            expected_reports: [
                [KC_LSHIFT, [0, 0, 0, 0, 0, 0]],        // press shift
                [KC_LSHIFT, [kc_to_u8!(A), 0, 0, 0, 0, 0]], // press shift + A
                [KC_LSHIFT, [0, 0, 0, 0, 0, 0]],        // release A
                [0, [0, 0, 0, 0, 0, 0]],           // release shift
                [0, [kc_to_u8!(B), 0, 0, 0, 0, 0]], // press B
                [0, [0, 0, 0, 0, 0, 0]],           // release B
            ]
        );
    }

    #[test]
    fn test_macro_with_delay() {
        let macro_sequences = &[Vec::from_slice(&[
            MacroOperation::Tap(HidKeyCode::A),
            MacroOperation::Delay(50 << 8), // 50 ms
            MacroOperation::Tap(HidKeyCode::B),
        ])
        .expect("too many elements")];

        let macro_data = define_macro_sequences(macro_sequences);
        let mut config = BehaviorConfig::default();
        config.keyboard_macros.macro_sequences = macro_data;

        let keyboard = create_simple_macro_keyboard(config);

        key_sequence_test!(
            keyboard: keyboard,
            sequence: [
                [0, 0, true, 0],
                [0, 0, false, 100],
            ],
            expected_reports: [
                [0, [kc_to_u8!(A), 0, 0, 0, 0, 0]], // press A
                [0, [0, 0, 0, 0, 0, 0]],            // release A
                // Delay 50 ms
                [0, [kc_to_u8!(B), 0, 0, 0, 0, 0]], // press B
                [0, [0, 0, 0, 0, 0, 0]],            // release B
            ]
        );
    }
}
