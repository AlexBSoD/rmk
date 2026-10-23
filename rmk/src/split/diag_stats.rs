//! Power diagnostics of a split peripheral, for tracking down battery drain
//! without a current shunt.
//!
//! A peripheral collects its battery voltage, regulator and reset state, BLE
//! link state, activity counters and (with `diag_cpu`) CPU active time, and
//! sends a [`DiagStats`] snapshot to the central on every battery refresh,
//! right after its `SleepStats`. The central keeps the last snapshot per
//! peripheral for the host to read over Vial.

use core::cell::Cell;
use core::sync::atomic::{AtomicI8, AtomicU8, AtomicU32, Ordering};

use embassy_sync::blocking_mutex::Mutex;
use embassy_time::Instant;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::RawMutex;

pub(crate) const DIAG_STATS_VERSION: u8 = 1;

/// The last battery sample measured a voltage.
pub(crate) const FLAG_BATTERY_SAMPLED: u8 = 0x01;
/// VBUS was present at the last battery sample. A charging half skips the
/// voltage measurement, so `battery_mv` is then the last discharging sample.
pub(crate) const FLAG_VBUS: u8 = 0x02;
/// `POWER.DCDCEN`: REG1 runs on the DC/DC converter instead of the LDO.
pub(crate) const FLAG_DCDC_REG1: u8 = 0x04;
/// `POWER.DCDCEN0`: REG0 runs on the DC/DC converter.
pub(crate) const FLAG_DCDC_REG0: u8 = 0x08;
/// `POWER.MAINREGSTATUS`: the chip is supplied on VDDH (high voltage mode).
pub(crate) const FLAG_HIGH_VOLTAGE: u8 = 0x10;
/// `cpu_active_ms` is measured; without `diag_cpu` it stays 0.
pub(crate) const FLAG_CPU_COUNTER: u8 = 0x20;

/// One snapshot of a peripheral's power diagnostics. Counters count since
/// boot and wrap; `uptime_ms` and `cpu_active_ms` wrap after 49.7 days.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct DiagStats {
    pub(crate) version: u8,
    /// `FLAG_*` bits.
    pub(crate) flags: u8,
    /// Last measured battery voltage; 0 before the first measurement.
    pub(crate) battery_mv: u16,
    /// `POWER.RESETREAS` as captured at boot: why this boot happened.
    pub(crate) reset_reason: u32,
    /// Boots since power-on: 1 after power loss, more after soft resets.
    pub(crate) boot_count: u16,
    pub(crate) uptime_ms: u32,
    /// Time the CPU spent out of sleep.
    pub(crate) cpu_active_ms: u32,
    pub(crate) key_presses: u32,
    pub(crate) rssi: i8,
    pub(crate) last_disconnect_reason: u8,
    pub(crate) disconnects: u16,
    /// Connection interval of the split link as applied by the controller;
    /// 0 between sessions.
    pub(crate) conn_interval_us: u32,
    pub(crate) peripheral_latency: u16,
    pub(crate) supervision_timeout_ms: u16,
    /// Connection parameter updates since boot, not counting the initial ones.
    pub(crate) param_updates: u16,
    pub(crate) split_tx: u32,
    pub(crate) split_rx: u32,
}

// A snapshot must fit one notification after the MTU exchange: the packet MTU
// minus the L2CAP and ATT headers, with one byte for the SplitMessage tag.
const _: () = core::assert!(DiagStats::POSTCARD_MAX_SIZE + 1 <= trouble_host::config::DEFAULT_PACKET_POOL_MTU - 7);

#[derive(Clone, Copy)]
struct State {
    battery_mv: u16,
    battery_flags: u8,
    reset_reason: u32,
    boot_count: u16,
    disconnects: u16,
    conn_interval_us: u32,
    peripheral_latency: u16,
    supervision_timeout_ms: u16,
    param_updates: u16,
    cpu: CpuTime,
}

static STATE: Mutex<RawMutex, Cell<State>> = Mutex::new(Cell::new(State {
    battery_mv: 0,
    battery_flags: 0,
    reset_reason: 0,
    boot_count: 0,
    disconnects: 0,
    conn_interval_us: 0,
    peripheral_latency: 0,
    supervision_timeout_ms: 0,
    param_updates: 0,
    cpu: CpuTime { last: 0, cycles: 0 },
}));

/// Key presses sent to the central.
pub(crate) static KEY_PRESSES: AtomicU32 = AtomicU32::new(0);
/// Split messages notified to the central.
pub(crate) static SPLIT_TX: AtomicU32 = AtomicU32::new(0);
/// Split messages received from the central.
pub(crate) static SPLIT_RX: AtomicU32 = AtomicU32::new(0);
/// Last RSSI of the split link in dBm, as read by the link health check;
/// starts at 127, "not available" in the HCI specification.
pub(crate) static RSSI: AtomicI8 = AtomicI8::new(127);
/// HCI reason of the last disconnect the controller reported; 0 for none.
pub(crate) static LAST_DISCONNECT_REASON: AtomicU8 = AtomicU8::new(0);

static PERIPHERAL_DIAG_STATS: Mutex<RawMutex, Cell<[Option<DiagStats>; crate::SPLIT_PERIPHERALS_NUM]>> =
    Mutex::new(Cell::new([None; crate::SPLIT_PERIPHERALS_NUM]));

fn update(f: impl FnOnce(&mut State)) {
    STATE.lock(|cell| {
        let mut state = cell.get();
        f(&mut state);
        cell.set(state);
    });
}

/// Boot record kept in RAM that is not zeroed at startup, so it survives a soft
/// or watchdog reset but not a power loss. `check` catches RAM overwritten in
/// between (by the bootloader's stack, for instance), which then reads as a
/// power-on.
#[derive(Clone, Copy)]
#[repr(C)]
struct BootRecord {
    magic: u32,
    count: u32,
    check: u32,
}

const BOOT_MAGIC: u32 = 0x5155_4245;

impl BootRecord {
    fn next(self) -> Self {
        let count = if self.magic == BOOT_MAGIC && self.check == !self.count {
            self.count.wrapping_add(1)
        } else {
            1
        };
        Self {
            magic: BOOT_MAGIC,
            count,
            check: !count,
        }
    }
}

/// cortex-m-rt's linker script puts `.uninit.*` in a NOLOAD section.
#[cfg(all(target_arch = "arm", target_os = "none"))]
#[unsafe(link_section = ".uninit.rmk_diag_boot")]
static mut BOOT_RECORD: core::mem::MaybeUninit<BootRecord> = core::mem::MaybeUninit::uninit();

/// Capture this boot's reset reason and boot count, and start the cycle
/// counter. Call once, before anything else reads `RESETREAS`.
pub(crate) fn capture_boot() {
    #[cfg(feature = "nrf52840_ble")]
    let reset_reason = {
        let power = embassy_nrf::pac::POWER;
        let reason = power.resetreas().read();
        // RESETREAS accumulates until cleared, and neither the Adafruit
        // bootloader nor the HAL clears it: write the bits back so the next
        // boot reports only its own reason.
        power.resetreas().write_value(reason);
        reason.0
    };
    #[cfg(not(feature = "nrf52840_ble"))]
    let reset_reason = 0;

    #[cfg(all(target_arch = "arm", target_os = "none"))]
    let boot_count = {
        // Safety: runs once at startup, before any other access; the record is
        // plain integers, so any RAM content is a value `next` can judge.
        let record = unsafe { core::ptr::read_volatile((&raw const BOOT_RECORD).cast::<BootRecord>()) }.next();
        unsafe { core::ptr::write_volatile((&raw mut BOOT_RECORD).cast::<BootRecord>(), record) };
        record.count
    };
    #[cfg(not(all(target_arch = "arm", target_os = "none")))]
    let boot_count = 0u32;

    #[cfg(all(feature = "diag_cpu", target_arch = "arm", target_os = "none"))]
    {
        // TRCENA powers the debug trace block, which may itself draw current;
        // that is why the cycle counter sits behind its own feature.
        // Safety: nothing else in the firmware uses DCB or DWT.
        let mut peripherals = unsafe { cortex_m::Peripherals::steal() };
        peripherals.DCB.enable_trace();
        peripherals.DWT.enable_cycle_counter();
    }

    let now = cycle_count();
    update(|state| {
        state.reset_reason = reset_reason;
        state.boot_count = boot_count.min(u32::from(u16::MAX)) as u16;
        state.cpu = CpuTime { last: now, cycles: 0 };
    });
}

/// Record a battery sample. `mv` is `None` when the measurement was skipped or
/// failed; the last measured voltage is kept then.
pub fn record_battery(mv: Option<u16>, vbus: bool) {
    update(|state| {
        state.battery_flags = 0;
        if let Some(mv) = mv {
            state.battery_mv = mv;
            state.battery_flags |= FLAG_BATTERY_SAMPLED;
        }
        if vbus {
            state.battery_flags |= FLAG_VBUS;
        }
    });
}

/// Record the split link parameters, at connection or after an update.
pub(crate) fn record_link_params(
    conn_interval_us: u64,
    peripheral_latency: u16,
    supervision_timeout_ms: u64,
    is_update: bool,
) {
    update(|state| {
        state.conn_interval_us = conn_interval_us.min(u64::from(u32::MAX)) as u32;
        state.peripheral_latency = peripheral_latency;
        state.supervision_timeout_ms = supervision_timeout_ms.min(u64::from(u16::MAX)) as u16;
        if is_update {
            state.param_updates = state.param_updates.wrapping_add(1);
        }
    });
}

/// Record the end of a split session, whatever ended it.
pub(crate) fn record_disconnect() {
    update(|state| {
        state.disconnects = state.disconnects.wrapping_add(1);
        state.conn_interval_us = 0;
        state.peripheral_latency = 0;
        state.supervision_timeout_ms = 0;
    });
}

/// CPU cycles accumulated from the 32-bit DWT cycle counter.
#[derive(Clone, Copy)]
struct CpuTime {
    last: u32,
    cycles: u64,
}

impl CpuTime {
    /// Add the cycles since the last sample. Correct across one wrap of the
    /// counter, so samples must come less than 2^32 active cycles apart.
    fn sample(self, now: u32) -> Self {
        Self {
            last: now,
            cycles: self.cycles + u64::from(now.wrapping_sub(self.last)),
        }
    }
}

const CPU_HZ: u64 = 64_000_000;

fn cycle_count() -> u32 {
    #[cfg(all(feature = "diag_cpu", target_arch = "arm", target_os = "none"))]
    {
        cortex_m::peripheral::DWT::cycle_count()
    }
    #[cfg(not(all(feature = "diag_cpu", target_arch = "arm", target_os = "none")))]
    {
        0
    }
}

/// Fold the cycle counter into the 64-bit total often enough that it cannot
/// wrap twice in between: at 64 MHz it wraps after 67 s of active time, and it
/// stands still while the core sleeps in WFE.
#[cfg(feature = "diag_cpu")]
pub(crate) async fn track_cpu_time() -> ! {
    loop {
        embassy_time::Timer::after_secs(30).await;
        let now = cycle_count();
        update(|state| state.cpu = state.cpu.sample(now));
    }
}

/// This peripheral's diagnostics right now.
pub(crate) fn current_diag_stats() -> DiagStats {
    let now = cycle_count();
    update(|state| state.cpu = state.cpu.sample(now));
    let state = STATE.lock(Cell::get);

    let mut flags = state.battery_flags;
    #[cfg(feature = "nrf52840_ble")]
    {
        let power = embassy_nrf::pac::POWER;
        if power.dcdcen().read().dcdcen() {
            flags |= FLAG_DCDC_REG1;
        }
        if power.dcdcen0().read().dcdcen() {
            flags |= FLAG_DCDC_REG0;
        }
        if power.mainregstatus().read().0 & 1 != 0 {
            flags |= FLAG_HIGH_VOLTAGE;
        }
    }
    if cfg!(all(feature = "diag_cpu", target_arch = "arm", target_os = "none")) {
        flags |= FLAG_CPU_COUNTER;
    }

    DiagStats {
        version: DIAG_STATS_VERSION,
        flags,
        battery_mv: state.battery_mv,
        reset_reason: state.reset_reason,
        boot_count: state.boot_count,
        uptime_ms: Instant::now().as_millis() as u32,
        cpu_active_ms: (state.cpu.cycles * 1000 / CPU_HZ) as u32,
        key_presses: KEY_PRESSES.load(Ordering::Relaxed),
        rssi: RSSI.load(Ordering::Relaxed),
        last_disconnect_reason: LAST_DISCONNECT_REASON.load(Ordering::Relaxed),
        disconnects: state.disconnects,
        conn_interval_us: state.conn_interval_us,
        peripheral_latency: state.peripheral_latency,
        supervision_timeout_ms: state.supervision_timeout_ms,
        param_updates: state.param_updates,
        split_tx: SPLIT_TX.load(Ordering::Relaxed),
        split_rx: SPLIT_RX.load(Ordering::Relaxed),
    }
}

pub(crate) fn update_peripheral_diag_stats(id: usize, stats: DiagStats) {
    PERIPHERAL_DIAG_STATS.lock(|cell| {
        let mut all = cell.get();
        if let Some(slot) = all.get_mut(id) {
            *slot = Some(stats);
            cell.set(all);
        }
    });
}

pub(crate) fn peripheral_diag_stats(id: usize) -> Option<DiagStats> {
    PERIPHERAL_DIAG_STATS.lock(|cell| cell.get().get(id).copied().flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_time_survives_a_cycle_counter_wrap() {
        let cpu = CpuTime {
            last: u32::MAX - 99,
            cycles: 1_000,
        };
        let cpu = cpu.sample(50);
        assert_eq!(cpu.last, 50);
        assert_eq!(cpu.cycles, 1_000 + 150);
        assert_eq!(cpu.sample(50).cycles, 1_150);
    }

    #[test]
    fn boot_count_restarts_on_garbage_and_counts_soft_resets() {
        let garbage = BootRecord {
            magic: 0xdead_beef,
            count: 7,
            check: 0,
        };
        let first = garbage.next();
        assert_eq!(first.count, 1);
        assert_eq!(first.next().next().count, 3);

        let mut torn = first.next();
        torn.count = 9;
        assert_eq!(torn.next().count, 1);
    }

    #[test]
    fn battery_keeps_the_last_voltage_while_charging() {
        record_battery(Some(3_870), false);
        record_battery(None, true);
        let stats = current_diag_stats();
        assert_eq!(stats.battery_mv, 3_870);
        assert_eq!(stats.flags & (FLAG_BATTERY_SAMPLED | FLAG_VBUS), FLAG_VBUS);
    }

    #[test]
    fn link_state_follows_sessions() {
        record_link_params(30_000, 0, 4_000, false);
        record_link_params(15_000, 9, 6_000, true);
        let stats = current_diag_stats();
        assert_eq!((stats.conn_interval_us, stats.peripheral_latency), (15_000, 9));
        assert_eq!(stats.supervision_timeout_ms, 6_000);
        assert_eq!(stats.param_updates, 1);

        record_disconnect();
        let stats = current_diag_stats();
        assert_eq!(stats.disconnects, 1);
        assert_eq!(stats.conn_interval_us, 0);
    }

    #[test]
    fn central_keeps_the_last_snapshot_per_peripheral() {
        let stats = current_diag_stats();
        let last = crate::SPLIT_PERIPHERALS_NUM - 1;
        assert_eq!(peripheral_diag_stats(last), None);
        update_peripheral_diag_stats(last, stats);
        update_peripheral_diag_stats(crate::SPLIT_PERIPHERALS_NUM, stats);
        assert_eq!(peripheral_diag_stats(last), Some(stats));
        assert_eq!(peripheral_diag_stats(crate::SPLIT_PERIPHERALS_NUM), None);
    }
}
