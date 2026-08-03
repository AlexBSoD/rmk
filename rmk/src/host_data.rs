use core::cell::RefCell;
use core::sync::atomic::{AtomicU8, Ordering};

use embassy_sync::blocking_mutex::Mutex;
use embassy_time::{Duration, Instant};

use crate::RawMutex;

const UNKNOWN: u8 = u8::MAX;
const HOST_TEXT_LEN: usize = 32;
/// The agent feed is pushed by a host daemon that can die or lose the socket it
/// watches. Without an expiry the display would keep showing a stale "1 working"
/// forever, so treat anything older than this as "no data".
const AGENTS_TTL: Duration = Duration::from_secs(30);

static HOST_HOUR: AtomicU8 = AtomicU8::new(UNKNOWN);
static HOST_MINUTE: AtomicU8 = AtomicU8::new(UNKNOWN);
static HOST_LAYOUT: AtomicU8 = AtomicU8::new(UNKNOWN);
static HOST_MEDIA_ARTIST: Mutex<RawMutex, RefCell<heapless::String<HOST_TEXT_LEN>>> =
    Mutex::new(RefCell::new(heapless::String::new()));
static HOST_MEDIA_TITLE: Mutex<RawMutex, RefCell<heapless::String<HOST_TEXT_LEN>>> =
    Mutex::new(RefCell::new(heapless::String::new()));
static HOST_AGENTS: Mutex<RawMutex, RefCell<Option<AgentSnapshot>>> = Mutex::new(RefCell::new(None));

/// How many host-side coding agents sit in each state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AgentSummary {
    pub working: u8,
    pub idle: u8,
    pub blocked: u8,
    pub done: u8,
    pub unknown: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AgentSnapshot {
    summary: AgentSummary,
    received: Instant,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostData {
    pub hour: Option<u8>,
    pub minute: Option<u8>,
    pub layout: Option<u8>,
    pub media_artist: heapless::String<HOST_TEXT_LEN>,
    pub media_title: heapless::String<HOST_TEXT_LEN>,
    /// `None` while no agent packet has arrived, or once the last one expired.
    pub agents: Option<AgentSummary>,
}

pub fn update_time(hour: u8, minute: u8) {
    if hour < 24 && minute < 60 {
        HOST_HOUR.store(hour, Ordering::Relaxed);
        HOST_MINUTE.store(minute, Ordering::Relaxed);
    } else {
        clear_time();
    }
}

pub fn update_layout(layout: u8) {
    HOST_LAYOUT.store(layout, Ordering::Relaxed);
}

pub fn clear_time() {
    HOST_HOUR.store(UNKNOWN, Ordering::Relaxed);
    HOST_MINUTE.store(UNKNOWN, Ordering::Relaxed);
}

pub fn update_media_artist(value: &str) {
    update_media_text(&HOST_MEDIA_ARTIST, value);
}

pub fn update_media_title(value: &str) {
    update_media_text(&HOST_MEDIA_TITLE, value);
}

pub fn update_agents(summary: AgentSummary) {
    let snapshot = AgentSnapshot {
        summary,
        received: Instant::now(),
    };
    HOST_AGENTS.lock(|cell| cell.replace(Some(snapshot)));
}

pub fn snapshot() -> HostData {
    HostData {
        hour: known(HOST_HOUR.load(Ordering::Relaxed)),
        minute: known(HOST_MINUTE.load(Ordering::Relaxed)),
        layout: known(HOST_LAYOUT.load(Ordering::Relaxed)),
        media_artist: HOST_MEDIA_ARTIST.lock(|cell| cell.borrow().clone()),
        media_title: HOST_MEDIA_TITLE.lock(|cell| cell.borrow().clone()),
        agents: HOST_AGENTS.lock(|cell| {
            let snapshot = (*cell.borrow())?;
            (snapshot.received.elapsed() < AGENTS_TTL).then_some(snapshot.summary)
        }),
    }
}

fn known(value: u8) -> Option<u8> {
    (value != UNKNOWN).then_some(value)
}

fn update_media_text(slot: &Mutex<RawMutex, RefCell<heapless::String<HOST_TEXT_LEN>>>, value: &str) {
    slot.lock(|cell| {
        let mut text = cell.borrow_mut();
        text.clear();
        for ch in value.chars() {
            if text.push(ch).is_err() {
                break;
            }
        }
    });
}
