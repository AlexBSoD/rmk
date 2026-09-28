//! Split status and half batteries on the XIAO's onboard RGB LED.
//!
//! The LED is common-anode (active low) on P0.26 (red), P0.30 (green) and
//! P0.06 (blue), driven by three channels of one PWM instance so it can be
//! dimmed — at full duty it is far too bright for a desk.
//!
//! - searching for the halves: short blue blip once a second;
//! - every half connected: solid green for one second, then dark;
//! - search window over (`Idle`): dark;
//! - User39 (battery level): left half as one long flash, right half as two
//!   short ones, coloured by level — green ≥ 50 %, yellow ≥ 20 %, red below,
//!   magenta when the half is not connected or has not reported yet.

use embassy_nrf::pwm::{DutyCycle, SimplePwm};
use embassy_time::{Duration, Instant};
use rmk::event::{
    PeripheralBatteryEvent, PeripheralBatteryRefreshEvent, PeripheralConnectedEvent, SplitConnectionState,
    SplitConnectionStateEvent,
};
use rmk::macros::processor;
use rmk::types::battery::BatteryStatus;

const HALVES: usize = 2;
/// PWM counter top; colours below are duties out of this.
pub const MAX_DUTY: u16 = 255;
const DIM: u16 = 24;

const SEARCH_PERIOD_MS: u64 = 1_000;
const SEARCH_ON_MS: u64 = 60;
const CONNECTED_MS: u64 = 1_000;
/// Give the halves time to answer the refresh before reading their levels.
const BATTERY_SETTLE_MS: u64 = 400;

type Color = [u16; 3];

const OFF: Color = [0, 0, 0];
const RED: Color = [DIM, 0, 0];
const YELLOW: Color = [DIM, DIM * 2 / 3, 0];
const GREEN: Color = [0, DIM, 0];
const BLUE: Color = [0, 0, DIM];
const MAGENTA: Color = [DIM, 0, DIM];

/// Battery flash pattern after the settle delay, as `(half, on_ms, off_ms)`.
const BATTERY_PATTERN: [(usize, u64, u64); 3] = [(0, 700, 500), (1, 200, 150), (1, 200, 0)];

#[processor(
    subscribe = [
        SplitConnectionStateEvent,
        PeripheralConnectedEvent,
        PeripheralBatteryEvent,
        PeripheralBatteryRefreshEvent
    ],
    poll_interval = 20
)]
pub struct XiaoStatusLed {
    pwm: SimplePwm<'static>,
    shown: Option<Color>,
    split_state: SplitConnectionState,
    phase_started: Instant,
    connected_until: Option<Instant>,
    connected: [bool; HALVES],
    battery: [Option<u8>; HALVES],
    battery_started: Option<Instant>,
}

impl XiaoStatusLed {
    pub fn new(pwm: SimplePwm<'static>) -> Self {
        Self {
            pwm,
            shown: None,
            split_state: SplitConnectionState::Searching,
            phase_started: Instant::now(),
            connected_until: None,
            connected: [false; HALVES],
            battery: [None; HALVES],
            battery_started: None,
        }
    }

    async fn on_split_connection_state_event(&mut self, event: SplitConnectionStateEvent) {
        self.apply_split_state(event.0);
        self.render();
    }

    async fn on_peripheral_connected_event(&mut self, event: PeripheralConnectedEvent) {
        if let Some(slot) = self.connected.get_mut(event.id) {
            *slot = event.connected;
        }
    }

    async fn on_peripheral_battery_event(&mut self, event: PeripheralBatteryEvent) {
        if let Some(slot) = self.battery.get_mut(event.id) {
            *slot = match event.state.0 {
                BatteryStatus::Available { level, .. } => level,
                BatteryStatus::Unavailable => None,
            };
        }
    }

    async fn on_peripheral_battery_refresh_event(&mut self, _event: PeripheralBatteryRefreshEvent) {
        self.battery_started = Some(Instant::now());
        self.render();
    }

    async fn poll(&mut self) {
        self.render();
    }

    fn apply_split_state(&mut self, state: SplitConnectionState) {
        if self.split_state == state {
            return;
        }
        let now = Instant::now();
        self.split_state = state;
        self.phase_started = now;
        self.connected_until =
            (state == SplitConnectionState::Connected).then(|| now + Duration::from_millis(CONNECTED_MS));
    }

    fn render(&mut self) {
        // Split-state events are edge-triggered and may fire before this
        // processor subscribes; the sticky snapshot is authoritative.
        self.apply_split_state(rmk::event::current_split_connection_state());

        let now = Instant::now();
        let color = self.battery_color(now).unwrap_or_else(|| self.split_color(now));
        if self.shown == Some(color) {
            return;
        }
        self.shown = Some(color);
        for (channel, duty) in color.into_iter().enumerate() {
            self.pwm.set_duty(channel, DutyCycle::normal(duty));
        }
    }

    /// Colour of the running battery pattern, or `None` once it has finished.
    fn battery_color(&mut self, now: Instant) -> Option<Color> {
        let started = self.battery_started?;
        let mut elapsed = now.duration_since(started).as_millis();
        if elapsed < BATTERY_SETTLE_MS {
            return Some(OFF);
        }
        elapsed -= BATTERY_SETTLE_MS;

        for (half, on_ms, off_ms) in BATTERY_PATTERN {
            if elapsed < on_ms {
                return Some(self.level_color(half));
            }
            elapsed -= on_ms;
            if elapsed < off_ms {
                return Some(OFF);
            }
            elapsed -= off_ms;
        }
        self.battery_started = None;
        None
    }

    fn level_color(&self, half: usize) -> Color {
        // PeripheralConnectedEvent is published without backpressure, so a
        // missed edge must not paint a live half as absent: Connected means
        // every half is up regardless of what this processor saw.
        let connected = self.connected[half] || self.split_state == SplitConnectionState::Connected;
        match (connected, self.battery[half]) {
            (true, Some(level)) if level >= 50 => GREEN,
            (true, Some(level)) if level >= 20 => YELLOW,
            (true, Some(_)) => RED,
            _ => MAGENTA,
        }
    }

    fn split_color(&self, now: Instant) -> Color {
        match self.split_state {
            SplitConnectionState::Searching => {
                let elapsed_ms = now.duration_since(self.phase_started).as_millis();
                if elapsed_ms % SEARCH_PERIOD_MS < SEARCH_ON_MS { BLUE } else { OFF }
            }
            SplitConnectionState::Connected if self.connected_until.is_some_and(|until| now < until) => GREEN,
            _ => OFF,
        }
    }
}
