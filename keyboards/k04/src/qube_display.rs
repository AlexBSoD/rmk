//! Ergohaven Qube dongle display (ST7789V over SPI).
//!
//! Full landscape **280×240** UI without a full-frame RGB565 buffer
//! (~134 KiB would OOM / kill HID on nRF52840).
//!
//! Strategy: **stripe multipass**
//! - Logical size: 280×240 (full panel after Deg90)
//! - Physical FB: 280×48×2 ≈ 27 KiB (< EasyDMA MAXCNT 65535, RAM-safe)
//! - Each redraw: for each stripe → clip-draw full UI → SPI that stripe
//!
//! Pinout (`qube.overlay`):
//! SPI3 SCK=P1.11 MOSI=P1.10 · CS=P1.13 · DC=P0.28 · RST=P0.03 · BL=P0.02

use core::fmt::Write as _;

use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_nrf::peripherals::{P0_02, P0_03, P0_28, P1_10, P1_11, P1_13, SPI3};
use embassy_nrf::spim::{self, Spim};
use embassy_nrf::{Peri, interrupt};
use embassy_time::{Delay, Duration, Instant, Timer};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_8X13, FONT_9X15};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, PrimitiveStyleBuilder, Rectangle, RoundedRectangle};
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use lcd_async::interface::SpiInterface;
use lcd_async::models::ST7789;
use lcd_async::options::{ColorInversion, Orientation, Rotation};
use lcd_async::{Builder, Display as LcdDisplay};
use rmk::core_traits::Runnable;
use rmk::display::{DisplayRenderer, RenderContext};
use rmk::event::{
    BatteryStatusEvent, CentralConnectedEvent, ConnectionStatusChangeEvent, EventSubscriber, KeyboardEvent,
    LayerChangeEvent, PeripheralBatteryEvent, PeripheralConnectedEvent, SleepStateEvent, SubscribableEvent,
    WpmUpdateEvent,
};
use rmk::processor::Processor;
use rmk_types::battery::BatteryStatus;
use static_cell::StaticCell;
use u8g2_fonts::{U8g2TextStyle, fonts};

// --- Panel geometry ---------------------------------------------------------

pub const PANEL_NATIVE_W: usize = 240;
pub const PANEL_NATIVE_H: usize = 280;
const PANEL_ROTATION: Rotation = Rotation::Deg90;

/// Full landscape frame (after Deg90).
pub const SCREEN_W: usize = 280;
pub const SCREEN_H: usize = 240;

/// Stripe height: 280×48×2 = 26_880 B < 64 KiB EasyDMA, comfortable RAM.
const STRIPE_H: usize = 48;
const STRIPE_BYTES: usize = SCREEN_W * STRIPE_H * 2;

const BACKLIGHT_ACTIVE_HIGH: bool = true;
const UI_X_OFFSET: i32 = 5;
const SAFE_X: i32 = 18;
const SAFE_W: u32 = SCREEN_W as u32 - (SAFE_X as u32 * 2);
const PANEL_RADIUS: u32 = 14;
const BAR_RADIUS: u32 = 5;

/// Body band below the header: agent grid flanked by the two battery columns.
const BODY_Y: i32 = 50;
const BODY_H: u32 = 182;
/// Width of a per-half battery column. Position (far left / far right) is what
/// says which half a column belongs to, so they carry no L/R letter.
const SIDE_W: u32 = 28;
const SIDE_RADIUS: u32 = 10;
const LEFT_BAT_X: i32 = SAFE_X;
const RIGHT_BAT_X: i32 = SAFE_X + SAFE_W as i32 - SIDE_W as i32;
/// Gap between a battery column and the agent panel.
const SIDE_GAP: i32 = 6;
const AGENT_X: i32 = SAFE_X + SIDE_W as i32 + SIDE_GAP;
const AGENT_W: u32 = SAFE_W - (SIDE_W + SIDE_GAP as u32) * 2;
/// 2×2 grid of statuses; a cell is a title on top with its count under it.
const AGENT_CELL_W: i32 = AGENT_W as i32 / 2;
const AGENT_CELL_H: i32 = BODY_H as i32 / 2;
/// Title top and count centre, relative to the top of a cell.
const AGENT_LABEL_DY: i32 = 12;
const AGENT_COUNT_DY: i32 = 58;

const HEADER_DIRTY: DirtyRegion = DirtyRegion::range(12, 44);
/// Agents and batteries now share one band, so both repaint together.
const BODY_DIRTY: DirtyRegion = DirtyRegion::range(48, 234);

const COL_BG: Rgb565 = Rgb565::new(0, 2, 4);
const COL_FG: Rgb565 = Rgb565::new(29, 61, 30);
const COL_MUTED: Rgb565 = Rgb565::new(11, 24, 20);
const COL_DIM: Rgb565 = Rgb565::new(5, 12, 14);
const COL_ACCENT: Rgb565 = Rgb565::new(3, 38, 31);
const COL_YELLOW: Rgb565 = Rgb565::new(31, 50, 0);
const COL_RED: Rgb565 = Rgb565::new(31, 5, 5);
const COL_BAR_BG: Rgb565 = Rgb565::new(2, 7, 9);
const COL_BAR_FG: Rgb565 = Rgb565::new(3, 42, 30);
const COL_PANEL: Rgb565 = Rgb565::new(2, 6, 9);
const COL_PANEL_HI: Rgb565 = Rgb565::new(3, 9, 13);
const COL_BORDER: Rgb565 = Rgb565::new(5, 13, 16);
const COL_BORDER_DIM: Rgb565 = Rgb565::new(3, 8, 11);

type SpiDev = ExclusiveDevice<Spim<'static>, Output<'static>, NoDelay>;
type Di = SpiInterface<SpiDev, Output<'static>>;
type Panel = LcdDisplay<Di, ST7789, Output<'static>>;

#[derive(Clone, Copy)]
enum DirtyRegion {
    Full,
    Range { y0: u16, y1: u16 },
}

impl DirtyRegion {
    const fn range(y0: u16, y1: u16) -> Self {
        Self::Range { y0, y1 }
    }

    fn union(self, other: Self) -> Self {
        match (self, other) {
            (Self::Full, _) | (_, Self::Full) => Self::Full,
            (Self::Range { y0: a0, y1: a1 }, Self::Range { y0: b0, y1: b1 }) => Self::Range {
                y0: a0.min(b0),
                y1: a1.max(b1),
            },
        }
    }
}

// --- Stripe framebuffer (clip window into full screen) ----------------------

struct StripeLcd {
    display: Panel,
    buffer: &'static mut [u8; STRIPE_BYTES],
    /// Top of the active stripe in full-screen coordinates.
    band_y: u16,
    /// Height of the active stripe (≤ STRIPE_H), last stripe may be shorter.
    band_h: u16,
}

impl StripeLcd {
    fn set_band(&mut self, y: u16, h: u16) {
        self.band_y = y;
        self.band_h = h.min(STRIPE_H as u16);
    }

    fn clear_stripe(&mut self, color: Rgb565) {
        let c = color.into_storage().to_be_bytes();
        for pix in self.buffer.chunks_exact_mut(2) {
            pix[0] = c[0];
            pix[1] = c[1];
        }
    }

    fn put_pixel(&mut self, x: i32, y: i32, color: Rgb565) {
        let x = x + UI_X_OFFSET;
        if x < 0 || y < 0 {
            return;
        }
        let x = x as u32;
        let y = y as u32;
        if x >= SCREEN_W as u32 {
            return;
        }
        let by = self.band_y as u32;
        let bh = self.band_h as u32;
        if y < by || y >= by + bh {
            return;
        }
        let ly = (y - by) as usize;
        let lx = x as usize;
        let off = (ly * SCREEN_W + lx) * 2;
        if off + 1 >= self.buffer.len() {
            return;
        }
        let c = color.into_storage().to_be_bytes();
        self.buffer[off] = c[0];
        self.buffer[off + 1] = c[1];
    }

    async fn flush_band(&mut self) {
        let w = SCREEN_W as u16;
        let h = self.band_h;
        let y = self.band_y;
        // Only send used rows (last stripe may be shorter).
        let bytes = (SCREEN_W * h as usize) * 2;
        let slice = &self.buffer[..bytes];
        let _ = self.display.show_raw_data(0, y, w, h, slice).await;
    }
}

impl OriginDimensions for StripeLcd {
    fn size(&self) -> Size {
        Size::new(SCREEN_W as u32, SCREEN_H as u32)
    }
}

impl DrawTarget for StripeLcd {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Rgb565>>,
    {
        for Pixel(p, col) in pixels {
            self.put_pixel(p.x, p.y, col);
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Rgb565) -> Result<(), Self::Error> {
        let by = self.band_y as i32;
        let bh = self.band_h as i32;
        let band = Rectangle::new(Point::new(0, by), Size::new(SCREEN_W as u32, bh as u32));
        let isect = area.intersection(&band);
        if isect.is_zero_sized() {
            return Ok(());
        }
        // Fast path: full-width clear of the stripe
        if isect.top_left.x == 0
            && isect.size.width == SCREEN_W as u32
            && isect.top_left.y == by
            && isect.size.height == bh as u32
        {
            self.clear_stripe(color);
            return Ok(());
        }
        let x0 = isect.top_left.x;
        let y0 = isect.top_left.y;
        let x1 = x0 + isect.size.width as i32;
        let y1 = y0 + isect.size.height as i32;
        for y in y0..y1 {
            for x in x0..x1 {
                self.put_pixel(x, y, color);
            }
        }
        Ok(())
    }

    fn clear(&mut self, color: Rgb565) -> Result<(), Self::Error> {
        // Only clear the active stripe (multipass re-renders full UI per band).
        self.clear_stripe(color);
        Ok(())
    }
}

// --- Lazy init --------------------------------------------------------------

struct PendingPins {
    spi: Peri<'static, SPI3>,
    sck: Peri<'static, P1_11>,
    mosi: Peri<'static, P1_10>,
    cs: Peri<'static, P1_13>,
    dc: Peri<'static, P0_28>,
    rst: Peri<'static, P0_03>,
}

enum LcdState {
    Pending(PendingPins),
    Active(StripeLcd),
    Failed,
}

pub struct LazyQubeLcd<I> {
    state: LcdState,
    irq: I,
}

impl<I> LazyQubeLcd<I>
where
    I: interrupt::typelevel::Binding<<SPI3 as spim::Instance>::Interrupt, spim::InterruptHandler<SPI3>>
        + Copy
        + 'static,
{
    async fn ensure_init(&mut self) {
        let pins = match core::mem::replace(&mut self.state, LcdState::Failed) {
            LcdState::Pending(p) => p,
            LcdState::Active(d) => {
                self.state = LcdState::Active(d);
                return;
            }
            LcdState::Failed => return,
        };
        match try_init_lcd(pins, self.irq).await {
            Some(lcd) => {
                info!("ST7789 ready (full-screen stripe mode)");
                self.state = LcdState::Active(lcd);
            }
            None => {
                warn!("ST7789 init failed — HID keeps running");
                self.state = LcdState::Failed;
            }
        }
    }

    async fn present_dirty(&mut self, renderer: &mut QubeStatusRenderer, ctx: &RenderContext, dirty: DirtyRegion) {
        self.ensure_init().await;
        let LcdState::Active(lcd) = &mut self.state else {
            return;
        };

        let (y0, y1) = match dirty {
            DirtyRegion::Full => (0, SCREEN_H as u16),
            DirtyRegion::Range { y0, y1 } => (y0.min(SCREEN_H as u16), y1.min(SCREEN_H as u16)),
        };

        let mut y = y0;
        while y < y1 {
            let remaining = y1.saturating_sub(y);
            let h = remaining.min(STRIPE_H as u16);
            lcd.set_band(y, h);
            lcd.clear_stripe(COL_BG);
            // Re-run full UI; DrawTarget keeps only this stripe's pixels.
            renderer.render(ctx, lcd);
            lcd.flush_band().await;
            y = y.saturating_add(h);
        }
    }
}

impl<I> OriginDimensions for LazyQubeLcd<I> {
    fn size(&self) -> Size {
        Size::new(SCREEN_W as u32, SCREEN_H as u32)
    }
}

// DrawTarget on LazyQubeLcd only needed if something draws before present;
// multipass uses StripeLcd directly via present().

async fn try_init_lcd<I>(pins: PendingPins, irq: I) -> Option<StripeLcd>
where
    I: interrupt::typelevel::Binding<<SPI3 as spim::Instance>::Interrupt, spim::InterruptHandler<SPI3>>
        + Copy
        + 'static,
{
    let mut spi_cfg = spim::Config::default();
    spi_cfg.frequency = spim::Frequency::M8;
    let spim = Spim::new_txonly(pins.spi, irq, pins.sck, pins.mosi, spi_cfg);

    let cs = Output::new(pins.cs, Level::High, OutputDrive::Standard);
    let dc = Output::new(pins.dc, Level::Low, OutputDrive::Standard);
    let rst = Output::new(pins.rst, Level::High, OutputDrive::Standard);

    let spi_dev = ExclusiveDevice::new_no_delay(spim, cs).ok()?;
    let di = SpiInterface::new(spi_dev, dc);

    let mut delay = Delay;
    let display = Builder::new(ST7789, di)
        .reset_pin(rst)
        .display_size(PANEL_NATIVE_W as u16, PANEL_NATIVE_H as u16)
        .display_offset(0, 20)
        .invert_colors(ColorInversion::Inverted)
        .orientation(Orientation::new().rotate(PANEL_ROTATION))
        .init(&mut delay)
        .await
        .ok()?;

    static FB: StaticCell<[u8; STRIPE_BYTES]> = StaticCell::new();
    let buffer = FB.init([0; STRIPE_BYTES]);
    Some(StripeLcd {
        display,
        buffer,
        band_y: 0,
        band_h: STRIPE_H as u16,
    })
}

// --- Dongle screen processor (own event loop + multipass present) -----------

pub struct DongleScreen<I>
where
    I: interrupt::typelevel::Binding<<SPI3 as spim::Instance>::Interrupt, spim::InterruptHandler<SPI3>>
        + Copy
        + 'static,
{
    lcd: LazyQubeLcd<I>,
    backlight: Output<'static>,
    renderer: QubeStatusRenderer,
    ctx: RenderContext,
    last_host_data: rmk::host_data::HostData,
    last_layer_names_version: u8,
    last_render: Instant,
    pending: bool,
    dirty: DirtyRegion,
    min_interval: Duration,
}

pub fn create_processor<I>(
    spi: Peri<'static, SPI3>,
    sck: Peri<'static, P1_11>,
    mosi: Peri<'static, P1_10>,
    cs: Peri<'static, P1_13>,
    dc: Peri<'static, P0_28>,
    rst: Peri<'static, P0_03>,
    bl: Peri<'static, P0_02>,
    irq: I,
) -> DongleScreen<I>
where
    I: interrupt::typelevel::Binding<<SPI3 as spim::Instance>::Interrupt, spim::InterruptHandler<SPI3>>
        + Copy
        + 'static,
{
    let level = if BACKLIGHT_ACTIVE_HIGH { Level::High } else { Level::Low };
    let backlight = Output::new(bl, level, OutputDrive::Standard);
    let host_data = rmk::host_data::snapshot();

    DongleScreen {
        lcd: LazyQubeLcd {
            state: LcdState::Pending(PendingPins {
                spi,
                sck,
                mosi,
                cs,
                dc,
                rst,
            }),
            irq,
        },
        backlight,
        renderer: QubeStatusRenderer {
            host_data: host_data.clone(),
        },
        ctx: RenderContext::default(),
        last_host_data: host_data,
        last_layer_names_version: crate::layer_names::version(),
        last_render: Instant::from_ticks(0),
        pending: true,
        dirty: DirtyRegion::Full,
        min_interval: Duration::from_millis(80),
    }
}

impl<I> DongleScreen<I>
where
    I: interrupt::typelevel::Binding<<SPI3 as spim::Instance>::Interrupt, spim::InterruptHandler<SPI3>>
        + Copy
        + 'static,
{
    async fn redraw(&mut self) {
        self.sync_host_data();
        self.sync_layer_names();
        let now = Instant::now();
        if now.duration_since(self.last_render) < self.min_interval {
            self.pending = true;
            return;
        }
        self.lcd.present_dirty(&mut self.renderer, &self.ctx, self.dirty).await;
        self.ctx.key_press_latch = false;
        self.pending = false;
        self.dirty = DirtyRegion::Full;
        self.last_render = Instant::now();
    }

    fn request_redraw(&mut self) {
        self.pending = true;
        self.dirty = DirtyRegion::Full;
    }

    fn request_redraw_region(&mut self, dirty: DirtyRegion) {
        self.dirty = if self.pending { self.dirty.union(dirty) } else { dirty };
        self.pending = true;
    }

    fn set_backlight(&mut self, on: bool) {
        let level = if on == BACKLIGHT_ACTIVE_HIGH {
            Level::High
        } else {
            Level::Low
        };
        self.backlight.set_level(level);
    }

    fn sync_host_data(&mut self) {
        let host_data = rmk::host_data::snapshot();
        if host_data == self.last_host_data {
            return;
        }
        // The agent panel and the clock live in different bands; repainting
        // both on every clock minute would drag the whole frame through SPI.
        let dirty = match (
            host_data.agents != self.last_host_data.agents,
            host_data.hour != self.last_host_data.hour || host_data.minute != self.last_host_data.minute,
        ) {
            (true, true) => HEADER_DIRTY.union(BODY_DIRTY),
            (true, false) => BODY_DIRTY,
            _ => HEADER_DIRTY,
        };
        self.last_host_data = host_data.clone();
        self.renderer.host_data = host_data;
        self.request_redraw_region(dirty);
    }

    fn sync_layer_names(&mut self) {
        let version = crate::layer_names::version();
        if version != self.last_layer_names_version {
            self.last_layer_names_version = version;
            self.request_redraw_region(HEADER_DIRTY);
        }
    }
}

pub struct NeverEvent;
struct NeverSub;

impl EventSubscriber for NeverSub {
    type Event = NeverEvent;
    async fn next_event(&mut self) -> NeverEvent {
        core::future::pending().await
    }
}

impl<I> Runnable for DongleScreen<I>
where
    I: interrupt::typelevel::Binding<<SPI3 as spim::Instance>::Interrupt, spim::InterruptHandler<SPI3>>
        + Copy
        + 'static,
{
    async fn run(&mut self) -> ! {
        self.pending = true;
        self.sync_host_data();
        self.redraw().await;

        let mut layer_sub = LayerChangeEvent::subscriber();
        let mut wpm_sub = WpmUpdateEvent::subscriber();
        let mut key_sub = KeyboardEvent::subscriber();
        let mut sleep_sub = SleepStateEvent::subscriber();
        let mut bat_sub = BatteryStatusEvent::subscriber();
        let mut conn_sub = ConnectionStatusChangeEvent::subscriber();
        let mut peri_conn_sub = PeripheralConnectedEvent::subscriber();
        let mut peri_bat_sub = PeripheralBatteryEvent::subscriber();
        let mut central_sub = CentralConnectedEvent::subscriber();

        loop {
            // Wait for at least one event (or deferred redraw timer).
            if self.pending {
                let wait = self
                    .min_interval
                    .checked_sub(self.last_render.elapsed())
                    .unwrap_or(Duration::MIN);
                match select(
                    Timer::after(wait),
                    Self::next_any_or_host_tick(
                        &mut layer_sub,
                        &mut wpm_sub,
                        &mut key_sub,
                        &mut sleep_sub,
                        &mut bat_sub,
                        &mut conn_sub,
                        &mut peri_conn_sub,
                        &mut peri_bat_sub,
                        &mut central_sub,
                    ),
                )
                .await
                {
                    Either::First(_) => {}
                    Either::Second(ev) => {
                        self.apply(ev);
                    }
                }
            } else {
                let ev = Self::next_any_or_host_tick(
                    &mut layer_sub,
                    &mut wpm_sub,
                    &mut key_sub,
                    &mut sleep_sub,
                    &mut bat_sub,
                    &mut conn_sub,
                    &mut peri_conn_sub,
                    &mut peri_bat_sub,
                    &mut central_sub,
                )
                .await;
                self.apply(ev);
            }

            // Coalesce a burst of events that arrived during the previous
            // multipass present (layer MO + OSM mods, etc.) before redrawing.
            for _ in 0..16 {
                match select(
                    Timer::after(Duration::from_millis(0)),
                    Self::next_any(
                        &mut layer_sub,
                        &mut wpm_sub,
                        &mut key_sub,
                        &mut sleep_sub,
                        &mut bat_sub,
                        &mut conn_sub,
                        &mut peri_conn_sub,
                        &mut peri_bat_sub,
                        &mut central_sub,
                    ),
                )
                .await
                {
                    Either::First(_) => break,
                    Either::Second(ev) => self.apply(ev),
                }
            }

            if self.pending {
                self.redraw().await;
            }
        }
    }
}

/// Unified UI event for the dongle screen loop.
enum UiEv {
    Layer(LayerChangeEvent),
    Wpm(WpmUpdateEvent),
    Key(KeyboardEvent),
    Sleep(SleepStateEvent),
    Bat(BatteryStatusEvent),
    Conn(ConnectionStatusChangeEvent),
    PeriConn(PeripheralConnectedEvent),
    PeriBat(PeripheralBatteryEvent),
    Central(CentralConnectedEvent),
    HostDataTick,
}

impl<I> DongleScreen<I>
where
    I: interrupt::typelevel::Binding<<SPI3 as spim::Instance>::Interrupt, spim::InterruptHandler<SPI3>>
        + Copy
        + 'static,
{
    async fn next_any_or_host_tick(
        layer: &mut impl EventSubscriber<Event = LayerChangeEvent>,
        wpm: &mut impl EventSubscriber<Event = WpmUpdateEvent>,
        key: &mut impl EventSubscriber<Event = KeyboardEvent>,
        sleep: &mut impl EventSubscriber<Event = SleepStateEvent>,
        bat: &mut impl EventSubscriber<Event = BatteryStatusEvent>,
        conn: &mut impl EventSubscriber<Event = ConnectionStatusChangeEvent>,
        peri_conn: &mut impl EventSubscriber<Event = PeripheralConnectedEvent>,
        peri_bat: &mut impl EventSubscriber<Event = PeripheralBatteryEvent>,
        central: &mut impl EventSubscriber<Event = CentralConnectedEvent>,
    ) -> UiEv {
        match select(
            Timer::after(Duration::from_millis(250)),
            Self::next_any(layer, wpm, key, sleep, bat, conn, peri_conn, peri_bat, central),
        )
        .await
        {
            Either::First(_) => UiEv::HostDataTick,
            Either::Second(ev) => ev,
        }
    }

    async fn next_any(
        layer: &mut impl EventSubscriber<Event = LayerChangeEvent>,
        wpm: &mut impl EventSubscriber<Event = WpmUpdateEvent>,
        key: &mut impl EventSubscriber<Event = KeyboardEvent>,
        sleep: &mut impl EventSubscriber<Event = SleepStateEvent>,
        bat: &mut impl EventSubscriber<Event = BatteryStatusEvent>,
        conn: &mut impl EventSubscriber<Event = ConnectionStatusChangeEvent>,
        peri_conn: &mut impl EventSubscriber<Event = PeripheralConnectedEvent>,
        peri_bat: &mut impl EventSubscriber<Event = PeripheralBatteryEvent>,
        central: &mut impl EventSubscriber<Event = CentralConnectedEvent>,
    ) -> UiEv {
        // Nested select — a bit verbose but no heap / macro dependency.
        // Prefer input events; depth is fine for status UI.
        use embassy_futures::select::{Either3, select3};

        match select3(
            select3(layer.next_event(), wpm.next_event(), key.next_event()),
            select3(sleep.next_event(), bat.next_event(), conn.next_event()),
            select3(peri_conn.next_event(), peri_bat.next_event(), central.next_event()),
        )
        .await
        {
            Either3::First(Either3::First(e)) => UiEv::Layer(e),
            Either3::First(Either3::Second(e)) => UiEv::Wpm(e),
            Either3::First(Either3::Third(e)) => UiEv::Key(e),
            Either3::Second(Either3::First(e)) => UiEv::Sleep(e),
            Either3::Second(Either3::Second(e)) => UiEv::Bat(e),
            Either3::Second(Either3::Third(e)) => UiEv::Conn(e),
            Either3::Third(Either3::First(e)) => UiEv::PeriConn(e),
            Either3::Third(Either3::Second(e)) => UiEv::PeriBat(e),
            Either3::Third(Either3::Third(e)) => UiEv::Central(e),
        }
    }

    fn apply(&mut self, ev: UiEv) {
        // Keyboard matrix floods KeyboardEvent; UI doesn't show individual
        // keys — skip redraw for those so multipass can keep up with layer.
        let mut need_redraw = true;
        match ev {
            UiEv::Layer(e) => {
                self.ctx.layer = e.0;
                self.request_redraw_region(HEADER_DIRTY);
                need_redraw = false;
            }
            UiEv::Wpm(e) => {
                self.ctx.wpm = e.0;
                need_redraw = false;
            }
            UiEv::Key(e) => {
                self.ctx.key_pressed = e.pressed;
                if e.pressed {
                    self.ctx.key_press_latch = true;
                    self.set_backlight(true);
                }
                need_redraw = false;
            }
            UiEv::Sleep(e) => {
                self.ctx.sleeping = e.0;
                self.set_backlight(!e.0);
                if !e.0 {
                    self.request_redraw();
                }
                need_redraw = false;
            }
            UiEv::Bat(e) => {
                self.ctx.battery = e;
                self.request_redraw_region(BODY_DIRTY);
                need_redraw = false;
            }
            UiEv::Conn(e) => self.ctx.ble_status = e.0.ble,
            UiEv::PeriConn(e) => {
                if let Some(slot) = self.ctx.peripherals_connected.get_mut(e.id) {
                    *slot = e.connected;
                }
            }
            UiEv::PeriBat(e) => {
                if let Some(slot) = self.ctx.peripheral_batteries.get_mut(e.id) {
                    *slot = e.state;
                }
                self.request_redraw_region(BODY_DIRTY);
                need_redraw = false;
            }
            UiEv::Central(e) => self.ctx.central_connected = e.connected,
            UiEv::HostDataTick => {
                self.sync_host_data();
                self.sync_layer_names();
                need_redraw = false;
            }
        }
        if need_redraw {
            self.request_redraw();
        }
    }
}

impl<I> Processor for DongleScreen<I>
where
    I: interrupt::typelevel::Binding<<SPI3 as spim::Instance>::Interrupt, spim::InterruptHandler<SPI3>>
        + Copy
        + 'static,
{
    type Event = NeverEvent;
    fn subscriber() -> impl EventSubscriber<Event = NeverEvent> {
        NeverSub
    }
    async fn process(&mut self, _: NeverEvent) {}
    async fn process_loop(&mut self) -> ! {
        self.run().await
    }
}

// Silence unused DisplayDriver import path if needed — keep for future.

// --- Full-screen UI ---------------------------------------------------------
//
// Fixed zones (280x240) so nothing overlaps:
//   14..42            compact header (host clock + active layer)
//   50..232, x 18/234 per-half battery gauges, one on each side
//   50..232, centre   host agent summary — the reason to glance at this screen

pub struct QubeStatusRenderer {
    host_data: rmk::host_data::HostData,
}

impl DisplayRenderer<Rgb565> for QubeStatusRenderer {
    fn render<D: DrawTarget<Color = Rgb565>>(&mut self, ctx: &RenderContext, display: &mut D) {
        let _ = display.clear(COL_BG);

        let clock = MonoTextStyle::new(&FONT_9X15, COL_FG);
        let layer_title = U8g2TextStyle::new(fonts::u8g2_font_8x13_t_cyrillic, COL_ACCENT);
        let top = TextStyleBuilder::new().baseline(Baseline::Top).build();
        let tr = TextStyleBuilder::new()
            .alignment(Alignment::Right)
            .baseline(Baseline::Top)
            .build();
        let mc = TextStyleBuilder::new()
            .alignment(Alignment::Center)
            .baseline(Baseline::Middle)
            .build();
        let left = ctx.peripherals_connected.first().copied().unwrap_or(false);
        let right = ctx.peripherals_connected.get(1).copied().unwrap_or(false);
        let lp = battery_reading(ctx.peripheral_batteries.first().map(|b| b.0));
        let rp = battery_reading(ctx.peripheral_batteries.get(1).map(|b| b.0));
        let mut layer_name_buf = crate::layer_names::LayerNameString::new();
        let name = if crate::layer_names::copy_layer_name(ctx.layer, &mut layer_name_buf) {
            layer_name_buf.as_str()
        } else {
            layer_name(ctx.layer)
        };

        // Header: clock on the left, active layer on the right.
        draw_panel(display, SAFE_X, 14, SAFE_W, 28, COL_PANEL, COL_BORDER_DIM, PANEL_RADIUS);
        draw_round_fill(display, SAFE_X + 11, 23, 3, 10, 2, COL_ACCENT);
        let mut s: heapless::String<16> = heapless::String::new();
        push_host_time(&mut s, self.host_data.hour, self.host_data.minute);
        let _ = Text::with_text_style(&s, Point::new(SAFE_X + 22, 21), clock, top).draw(display);
        let _ =
            Text::with_text_style(name, Point::new(SAFE_X + SAFE_W as i32 - 14, 21), &layer_title, tr).draw(display);

        // Per-half battery gauges: left column = left half, right = right half.
        draw_bat_column(display, LEFT_BAT_X, lp, left);
        draw_bat_column(display, RIGHT_BAT_X, rp, right);

        // Agent summary: 2×2 grid of statuses, each a title with its count below.
        draw_panel(
            display,
            AGENT_X,
            BODY_Y,
            AGENT_W,
            BODY_H,
            COL_PANEL_HI,
            COL_BORDER_DIM,
            PANEL_RADIUS,
        );
        match self.host_data.agents {
            Some(agents) => {
                // Blocked agents are the only state worth interrupting typing
                // for, so they get the warning colour even at a glance.
                let cells = [
                    ("WORKING", agents.working, COL_ACCENT),
                    ("BLOCKED", agents.blocked, COL_YELLOW),
                    ("IDLE", agents.idle, COL_MUTED),
                    ("DONE", agents.done, COL_FG),
                ];
                // The cross splitting the panel into quadrants.
                let rule = PrimitiveStyle::with_fill(COL_BORDER_DIM);
                let _ = Rectangle::new(
                    Point::new(AGENT_X + 12, BODY_Y + AGENT_CELL_H),
                    Size::new(AGENT_W - 24, 1),
                )
                .into_styled(rule)
                .draw(display);
                let _ = Rectangle::new(
                    Point::new(AGENT_X + AGENT_CELL_W, BODY_Y + 12),
                    Size::new(1, BODY_H - 24),
                )
                .into_styled(rule)
                .draw(display);
                for (i, (label, count, accent)) in cells.iter().enumerate() {
                    let x0 = AGENT_X + (i as i32 % 2) * AGENT_CELL_W;
                    let y0 = BODY_Y + (i as i32 / 2) * AGENT_CELL_H;
                    draw_agent_cell(display, x0, y0, label, *count, *accent);
                }
            }
            None => {
                let offline = MonoTextStyle::new(&FONT_8X13, COL_DIM);
                let _ = Text::with_text_style("NO AGENT FEED", Point::new(SCREEN_W as i32 / 2, 141), offline, mc)
                    .draw(display);
            }
        }
    }
}

/// One quadrant of the agent grid: title on top, count below. The count is
/// dropped at zero so a screen with nothing running reads as quiet rather than
/// as data; only the dimmed title stays to keep the quadrant addressable.
fn draw_agent_cell<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    x0: i32,
    y0: i32,
    label: &str,
    count: u8,
    accent: Rgb565,
) {
    let cx = x0 + AGENT_CELL_W / 2;
    let tc = TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Top)
        .build();
    let mc = TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();

    let label_style = MonoTextStyle::new(&FONT_6X10, if count > 0 { accent } else { COL_DIM });
    let _ = Text::with_text_style(label, Point::new(cx, y0 + AGENT_LABEL_DY), label_style, tc).draw(display);

    if count == 0 {
        return;
    }
    let mut s: heapless::String<4> = heapless::String::new();
    let _ = write!(&mut s, "{}", count);
    let count_style = U8g2TextStyle::new(fonts::u8g2_font_logisoso32_tn, accent);
    let _ = Text::with_text_style(&s, Point::new(cx, y0 + AGENT_COUNT_DY), count_style, mc).draw(display);
}

fn draw_panel<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    fill: Rgb565,
    stroke: Rgb565,
    radius: u32,
) {
    let rect = Rectangle::new(Point::new(x, y), Size::new(w, h));
    let style = PrimitiveStyleBuilder::new()
        .fill_color(fill)
        .stroke_color(stroke)
        .stroke_width(1)
        .build();
    let _ = RoundedRectangle::with_equal_corners(rect, Size::new(radius, radius))
        .into_styled(style)
        .draw(display);
}

fn draw_round_fill<D: DrawTarget<Color = Rgb565>>(
    display: &mut D,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    radius: u32,
    fill: Rgb565,
) {
    let rect = Rectangle::new(Point::new(x, y), Size::new(w, h));
    let _ = RoundedRectangle::with_equal_corners(rect, Size::new(radius, radius))
        .into_styled(PrimitiveStyle::with_fill(fill))
        .draw(display);
}

#[derive(Clone, Copy)]
enum BatReading {
    Unknown,
    Pending,
    Pct(u8),
}

fn battery_reading(status: Option<BatteryStatus>) -> BatReading {
    match status {
        Some(BatteryStatus::Available { level: Some(level), .. }) => BatReading::Pct(level),
        Some(BatteryStatus::Available { level: None, .. }) => BatReading::Pending,
        Some(BatteryStatus::Unavailable) | None => BatReading::Unknown,
    }
}

/// Vertical gauge for one half, filling from the bottom, with the percentage
/// above it. Which half it reports is said by which side of the screen it is
/// on, so the column carries no L/R letter.
fn draw_bat_column<D: DrawTarget<Color = Rgb565>>(display: &mut D, x: i32, reading: BatReading, connected: bool) {
    let (label, col, fill_pct): (heapless::String<8>, Rgb565, Option<u8>) = match (connected, reading) {
        (false, _) => {
            let mut s = heapless::String::new();
            let _ = s.push_str("--");
            (s, COL_DIM, None)
        }
        (true, BatReading::Unknown) | (true, BatReading::Pending) => {
            let mut s = heapless::String::new();
            let _ = s.push_str("??");
            (s, COL_DIM, None)
        }
        (true, BatReading::Pct(p)) => {
            let mut s = heapless::String::new();
            let _ = write!(&mut s, "{}%", p);
            let c = if p < 10 {
                COL_RED
            } else if p < 25 {
                COL_YELLOW
            } else {
                COL_FG
            };
            (s, c, Some(p))
        }
    };

    draw_panel(
        display,
        x,
        BODY_Y,
        SIDE_W,
        BODY_H,
        COL_PANEL,
        COL_BORDER_DIM,
        SIDE_RADIUS,
    );

    let cx = x + SIDE_W as i32 / 2;
    let percent = MonoTextStyle::new(&FONT_6X10, col);
    let mc = TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();
    let _ = Text::with_text_style(&label, Point::new(cx, BODY_Y + 14), percent, mc).draw(display);

    // Track runs from just under the reading down to the bottom of the body.
    let bw = 10u32;
    let bx = cx - bw as i32 / 2;
    let by = BODY_Y + 28;
    let bh = (BODY_Y + BODY_H as i32 - 10 - by) as u32;
    let bar = RoundedRectangle::with_equal_corners(
        Rectangle::new(Point::new(bx, by), Size::new(bw, bh)),
        Size::new(BAR_RADIUS, BAR_RADIUS),
    );
    let bar_style = PrimitiveStyleBuilder::new()
        .fill_color(COL_BAR_BG)
        .stroke_color(COL_BORDER)
        .stroke_width(1)
        .build();
    let _ = bar.into_styled(bar_style).draw(display);
    // Terminal nub on top, so the column still reads as a battery.
    draw_round_fill(display, bx + 3, by - 4, 4, 3, 2, COL_BORDER_DIM);
    if let Some(pct) = fill_pct {
        if pct > 0 {
            let inner = bh - 4;
            let fh = (inner * pct as u32 / 100).max(2);
            let fc = if pct < 10 {
                COL_RED
            } else if pct < 25 {
                COL_YELLOW
            } else {
                COL_BAR_FG
            };
            draw_round_fill(display, bx + 2, by + 2 + (inner - fh) as i32, bw - 4, fh, 3, fc);
        }
    }
}

fn layer_name(layer: u8) -> &'static str {
    crate::DEFAULT_LAYER_NAMES.get(layer as usize).copied().unwrap_or("?")
}

fn push_host_time(buffer: &mut heapless::String<16>, hour: Option<u8>, minute: Option<u8>) {
    match (hour, minute) {
        (Some(hour), Some(minute)) => {
            let _ = write!(buffer, "{:02}:{:02}", hour, minute);
        }
        _ => {
            let _ = buffer.push_str("--:--");
        }
    }
}
