//! Sleep statistics of a split peripheral, for tracking down battery drain.
//!
//! A peripheral counts how often it wakes up and how long it stays awake, and
//! reports the totals to the central on every battery refresh. The central
//! keeps the last report per peripheral for the host to read.

use core::cell::Cell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_time::Instant;

use super::SleepStats;
use crate::RawMutex;

#[derive(Clone, Copy)]
struct Tracker {
    wakes: u16,
    awake_ms: u64,
    sleeping: bool,
    since: Instant,
}

// A peripheral boots awake.
static TRACKER: Mutex<RawMutex, Cell<Tracker>> = Mutex::new(Cell::new(Tracker {
    wakes: 0,
    awake_ms: 0,
    sleeping: false,
    since: Instant::MIN,
}));

static PERIPHERAL_SLEEP_STATS: Mutex<RawMutex, Cell<[Option<SleepStats>; crate::SPLIT_PERIPHERALS_NUM]>> =
    Mutex::new(Cell::new([None; crate::SPLIT_PERIPHERALS_NUM]));

/// Record a sleep state change on this peripheral.
pub(crate) fn record_sleep_state(sleeping: bool) {
    let now = Instant::now();
    TRACKER.lock(|cell| {
        let mut tracker = cell.get();
        if tracker.sleeping == sleeping {
            return;
        }
        if sleeping {
            tracker.awake_ms += now.duration_since(tracker.since).as_millis();
        } else {
            tracker.wakes = tracker.wakes.wrapping_add(1);
        }
        tracker.sleeping = sleeping;
        tracker.since = now;
        cell.set(tracker);
    });
}

/// Totals of this peripheral since boot, including the current awake stretch.
pub(crate) fn current_sleep_stats() -> SleepStats {
    let now = Instant::now();
    let tracker = TRACKER.lock(Cell::get);
    let mut awake_ms = tracker.awake_ms;
    if !tracker.sleeping {
        awake_ms += now.duration_since(tracker.since).as_millis();
    }
    SleepStats {
        wakes: tracker.wakes,
        awake_min: (awake_ms / 60_000).min(u64::from(u16::MAX)) as u16,
        uptime_min: (now.as_millis() / 60_000).min(u64::from(u16::MAX)) as u16,
    }
}

pub(crate) fn update_peripheral_sleep_stats(id: usize, stats: SleepStats) {
    PERIPHERAL_SLEEP_STATS.lock(|cell| {
        let mut all = cell.get();
        if let Some(slot) = all.get_mut(id) {
            *slot = Some(stats);
            cell.set(all);
        }
    });
}

pub(crate) fn peripheral_sleep_stats(id: usize) -> Option<SleepStats> {
    PERIPHERAL_SLEEP_STATS.lock(|cell| cell.get().get(id).copied().flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_wakes_only_on_real_transitions() {
        record_sleep_state(false);
        record_sleep_state(true);
        record_sleep_state(true);
        record_sleep_state(false);
        record_sleep_state(true);
        record_sleep_state(false);
        assert_eq!(current_sleep_stats().wakes, 2);
    }

    #[test]
    fn central_keeps_the_last_report_per_peripheral() {
        let stats = SleepStats {
            wakes: 7,
            awake_min: 30,
            uptime_min: 600,
        };
        let last = crate::SPLIT_PERIPHERALS_NUM - 1;
        assert_eq!(peripheral_sleep_stats(last), None);
        update_peripheral_sleep_stats(last, stats);
        update_peripheral_sleep_stats(crate::SPLIT_PERIPHERALS_NUM, stats);
        assert_eq!(peripheral_sleep_stats(last), Some(stats));
        assert_eq!(peripheral_sleep_stats(crate::SPLIT_PERIPHERALS_NUM), None);
    }
}
