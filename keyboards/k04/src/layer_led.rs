use embassy_nrf::pwm::{SequenceConfig, SequencePwm, SingleSequenceMode, SingleSequencer};
use embassy_time::{Duration, Instant, Timer};
use rmk::event::{
    BatteryStatusEvent, CentralConnectedEvent, ConnectionStatusChangeEvent, LayerChangeEvent,
    PeripheralBatteryRefreshEvent, PeripheralConnectedEvent, PeripheralSettingsEvent,
};
use rmk::macros::processor;
use rmk::types::battery::{BatteryStatus, ChargeState};
use rmk::types::ble::BleState;
use rmk::types::connection::ConnectionStatus;

use crate::module_settings::{self, Rgb};

const LED_COUNT: usize = 1;
const LOW_BATTERY_MAX: u8 = 20;
const CHARGED_BATTERY_MIN: u8 = 100;
const BATTERY_PULSE_INTERVAL_MS: u64 = 2_000;
const BATTERY_PULSE_ON_MS: u64 = 120;
const CONNECTED_PULSE_MS: u64 = 520;
const INDICATOR_DURATION_MS: u64 = 1_000;
const PWM_POLARITY_INVERTED: u16 = 0x8000;
const PWM_T0H: u16 = PWM_POLARITY_INVERTED | 6;
const PWM_T1H: u16 = PWM_POLARITY_INVERTED | 13;
const RESET_SLOTS: usize = 80;
const FRAME_WORDS: usize = LED_COUNT * 24 + RESET_SLOTS;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Overlay {
    Profile(u8),
    Advertising,
    HostConnected,
    SplitMissing,
    SplitConnected,
    Battery(u8),
    Charging,
    LowBattery,
}

#[derive(Clone, Copy)]
struct TimedOverlay {
    kind: Overlay,
    started: Instant,
    ends: Instant,
}

#[processor(
    subscribe = [
        LayerChangeEvent,
        ConnectionStatusChangeEvent,
        PeripheralConnectedEvent,
        CentralConnectedEvent,
        PeripheralSettingsEvent,
        BatteryStatusEvent,
        PeripheralBatteryRefreshEvent
    ],
    poll_interval = 20
)]
pub struct LayerLed {
    led: SequencePwm<'static>,
    current_layer: Option<u8>,
    layer_deadline: Option<Instant>,
    current_color: Option<Rgb>,
    connection_status: Option<ConnectionStatus>,
    ble_profile: u8,
    ble_state: BleState,
    deferred_ble_state: Option<BleState>,
    split_connected: bool,
    overlay: Option<TimedOverlay>,
    latest_battery: Option<u8>,
    battery_charging: bool,
    pending_battery_display: bool,
    usb_powered: bool,
    last_charging_pulse: Instant,
    last_low_battery_pulse: Instant,
}

impl LayerLed {
    pub fn new(led: SequencePwm<'static>) -> Self {
        let now = Instant::now();
        Self {
            led,
            current_layer: Some(0),
            layer_deadline: None,
            current_color: None,
            connection_status: None,
            ble_profile: 0,
            ble_state: BleState::Inactive,
            deferred_ble_state: None,
            split_connected: true,
            overlay: None,
            latest_battery: None,
            battery_charging: false,
            pending_battery_display: false,
            usb_powered: usb_vbus_detected(),
            last_charging_pulse: now,
            last_low_battery_pulse: now,
        }
    }

    async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
        let now = Instant::now();
        self.current_layer = Some(event.0);
        // A layer change is direct user feedback and must not remain hidden
        // behind a stale Bluetooth/split/battery indicator. Periodic battery
        // indications can start again after their normal interval.
        self.overlay = None;
        self.deferred_ble_state = None;
        self.arm_layer_timeout(now);
        self.render(now).await;
    }

    async fn on_connection_status_change_event(&mut self, event: ConnectionStatusChangeEvent) {
        let previous = self.connection_status;
        let repeated = previous == Some(event.0);
        let profile_changed = previous.is_some_and(|status| status.ble.profile != event.0.ble.profile);
        let state_changed = previous.is_none_or(|status| status.ble.state != event.0.ble.state);

        self.connection_status = Some(event.0);
        self.ble_profile = event.0.ble.profile;
        self.ble_state = event.0.ble.state;

        if profile_changed || repeated {
            self.deferred_ble_state = None;
            self.start_overlay(Overlay::Profile(self.ble_profile), indicator_duration());
        } else if state_changed {
            if self
                .overlay
                .is_some_and(|overlay| matches!(overlay.kind, Overlay::Profile(_)))
            {
                self.deferred_ble_state = Some(self.ble_state);
            } else {
                self.start_ble_overlay(self.ble_state);
            }
        }

        self.render(Instant::now()).await;
    }

    async fn on_peripheral_connected_event(&mut self, event: PeripheralConnectedEvent) {
        self.set_split_connected(event.connected);
        self.render(Instant::now()).await;
    }

    async fn on_central_connected_event(&mut self, event: CentralConnectedEvent) {
        self.set_split_connected(event.connected);
        self.render(Instant::now()).await;
    }

    async fn on_peripheral_settings_event(&mut self, event: PeripheralSettingsEvent) {
        module_settings::apply_settings_packet(&event.0);
        self.current_color = None;
        self.arm_layer_timeout(Instant::now());
        self.render(Instant::now()).await;
    }

    async fn on_battery_status_event(&mut self, event: BatteryStatusEvent) {
        if let BatteryStatus::Available { charge_state, level } = event.0 {
            self.latest_battery = level;
            self.battery_charging = charge_state == ChargeState::Charging;
            if self.pending_battery_display {
                if let Some(level) = level {
                    self.pending_battery_display = false;
                    self.start_overlay(Overlay::Battery(level), indicator_duration());
                }
            }
        }
        self.render(Instant::now()).await;
    }

    async fn on_peripheral_battery_refresh_event(&mut self, _event: PeripheralBatteryRefreshEvent) {
        self.pending_battery_display = true;
        if let Some(level) = self.latest_battery {
            self.pending_battery_display = false;
            self.start_overlay(Overlay::Battery(level), indicator_duration());
            self.render(Instant::now()).await;
        }
    }

    async fn poll(&mut self) {
        let now = Instant::now();
        let usb_powered = usb_vbus_detected();
        if usb_powered != self.usb_powered {
            self.usb_powered = usb_powered;
            if usb_powered {
                self.last_charging_pulse = now;
                self.start_overlay(Overlay::Charging, Duration::from_millis(BATTERY_PULSE_ON_MS));
            } else if self.overlay.is_some_and(|overlay| overlay.kind == Overlay::Charging) {
                self.overlay = None;
                self.arm_layer_timeout(now);
            }
        }

        if self.overlay.is_some_and(|overlay| now >= overlay.ends) {
            let expired = self.overlay.take().map(|overlay| overlay.kind);
            if matches!(expired, Some(Overlay::Profile(_))) {
                if let Some(state) = self.deferred_ble_state.take() {
                    self.start_ble_overlay(state);
                } else {
                    self.arm_layer_timeout(now);
                }
            } else {
                self.arm_layer_timeout(now);
            }
        }

        if self.overlay.is_none() {
            if self.is_charging()
                && now.duration_since(self.last_charging_pulse).as_millis() >= BATTERY_PULSE_INTERVAL_MS
            {
                self.last_charging_pulse = now;
                self.start_overlay(Overlay::Charging, Duration::from_millis(BATTERY_PULSE_ON_MS));
            } else if !self.is_charging()
                && self.latest_battery.is_some_and(|level| level <= LOW_BATTERY_MAX)
                && now.duration_since(self.last_low_battery_pulse).as_millis() >= BATTERY_PULSE_INTERVAL_MS
            {
                self.last_low_battery_pulse = now;
                self.start_overlay(Overlay::LowBattery, Duration::from_millis(BATTERY_PULSE_ON_MS));
            }
        }

        self.render(now).await;
    }

    fn set_split_connected(&mut self, connected: bool) {
        if self.split_connected == connected {
            return;
        }
        self.split_connected = connected;
        if connected {
            self.start_overlay(Overlay::SplitConnected, Duration::from_millis(CONNECTED_PULSE_MS));
        } else {
            self.start_overlay(Overlay::SplitMissing, indicator_duration());
        }
    }

    fn start_ble_overlay(&mut self, state: BleState) {
        match state {
            BleState::Advertising => self.start_overlay(Overlay::Advertising, indicator_duration()),
            BleState::Connected => {
                self.start_overlay(Overlay::HostConnected, Duration::from_millis(CONNECTED_PULSE_MS));
            }
            BleState::Inactive => {
                if self
                    .overlay
                    .is_some_and(|overlay| matches!(overlay.kind, Overlay::Advertising | Overlay::HostConnected))
                {
                    self.overlay = None;
                    self.arm_layer_timeout(Instant::now());
                }
            }
        }
    }

    fn start_overlay(&mut self, kind: Overlay, duration: Duration) {
        let now = Instant::now();
        self.overlay = Some(TimedOverlay {
            kind,
            started: now,
            ends: now + duration,
        });
    }

    fn arm_layer_timeout(&mut self, now: Instant) {
        let timeout = module_settings::led_timeout_sec();
        self.layer_deadline = (timeout != 0).then(|| now + Duration::from_secs(u64::from(timeout)));
    }

    fn is_charging(&self) -> bool {
        self.battery_charging
            || (self.usb_powered && !self.latest_battery.is_some_and(|level| level >= CHARGED_BATTERY_MIN))
    }

    async fn render(&mut self, now: Instant) {
        let color = self
            .overlay
            .map(|overlay| overlay_color(overlay, now))
            .unwrap_or_else(|| self.layer_color(now));

        if self.current_color == Some(color) {
            return;
        }
        self.current_color = Some(color);
        send_color(&mut self.led, color).await;
    }

    fn layer_color(&self, now: Instant) -> Rgb {
        if self.layer_deadline.is_some_and(|deadline| now >= deadline) {
            return color_off();
        }
        self.current_layer.map(color_for_layer).unwrap_or_else(color_off)
    }
}

fn indicator_duration() -> Duration {
    Duration::from_millis(INDICATOR_DURATION_MS)
}

fn overlay_color(overlay: TimedOverlay, now: Instant) -> Rgb {
    let elapsed_ms = now.duration_since(overlay.started).as_millis();
    match overlay.kind {
        Overlay::Profile(profile) => color_for_bt_profile(profile),
        Overlay::Advertising => blink_color(color_blue(), elapsed_ms, 1_000, 500),
        Overlay::HostConnected => connected_pulse_color(color_green(), elapsed_ms),
        Overlay::SplitMissing => blink_color(color_yellow(), elapsed_ms, 1_500, 150),
        Overlay::SplitConnected => connected_pulse_color(color_yellow(), elapsed_ms),
        Overlay::Battery(level) => color_for_battery(level),
        Overlay::Charging => color_green(),
        Overlay::LowBattery => color_red(),
    }
}

fn connected_pulse_color(color: Rgb, elapsed_ms: u64) -> Rgb {
    match elapsed_ms {
        0..=100 | 180..=280 => color,
        _ => color_off(),
    }
}

fn blink_color(color: Rgb, elapsed_ms: u64, period_ms: u64, on_ms: u64) -> Rgb {
    if elapsed_ms % period_ms < on_ms {
        color
    } else {
        color_off()
    }
}

fn color_for_layer(layer: u8) -> Rgb {
    scale_color(module_settings::layer_color(layer))
}

fn color_for_bt_profile(profile: u8) -> Rgb {
    scale_color(module_settings::bt_profile_color(profile))
}

fn color_for_battery(level: u8) -> Rgb {
    let color = match level {
        0..=20 => Rgb { r: 255, g: 0, b: 0 },
        21..=40 => Rgb { r: 255, g: 80, b: 0 },
        41..=74 => Rgb { r: 255, g: 220, b: 0 },
        _ => Rgb { r: 0, g: 255, b: 0 },
    };
    scale_color(color)
}

fn color_blue() -> Rgb {
    scale_color(Rgb { r: 0, g: 0, b: 255 })
}

fn color_green() -> Rgb {
    scale_color(Rgb { r: 0, g: 255, b: 0 })
}

fn color_red() -> Rgb {
    scale_color(Rgb { r: 255, g: 0, b: 0 })
}

fn color_yellow() -> Rgb {
    scale_color(Rgb { r: 255, g: 180, b: 0 })
}

fn color_off() -> Rgb {
    Rgb { r: 0, g: 0, b: 0 }
}

fn scale_color(color: Rgb) -> Rgb {
    Rgb {
        r: scale(color.r),
        g: scale(color.g),
        b: scale(color.b),
    }
}

fn scale(value: u8) -> u8 {
    ((u16::from(value) * u16::from(module_settings::led_brightness())) / 255).min(255) as u8
}

fn usb_vbus_detected() -> bool {
    embassy_nrf::pac::POWER.usbregstatus().read().vbusdetect()
}

async fn send_color(led: &mut SequencePwm<'static>, color: Rgb) {
    let mut words = [0u16; FRAME_WORDS];
    let mut i = 0usize;

    for byte in [color.g, color.r, color.b] {
        for bit in (0..8).rev() {
            words[i] = if (byte & (1 << bit)) != 0 { PWM_T1H } else { PWM_T0H };
            i += 1;
        }
    }

    let sequencer = SingleSequencer::new(led, &words, SequenceConfig::default());
    let _ = sequencer.start(SingleSequenceMode::Times(1));
    Timer::after(Duration::from_micros(200)).await;
    sequencer.stop();
}
