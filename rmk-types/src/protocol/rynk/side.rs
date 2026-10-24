//! Personal Qube commands in `0x7EF0..=0x7EFF`, the range Ergohaven's table
//! leaves free: the host-agent window and the halves' diagnostics. No release
//! carries them, so neither the Ergohaven table nor its revision covers them; a
//! host asks `GetSideInfo` and takes `UnknownCmd` as "none of these".

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

#[cfg(test)]
use super::command::EndpointMeta;
use super::command::{Cmd, Endpoint, assert_unique, endpoints};
#[cfg(any(test, not(feature = "host")))]
use super::RynkError;

/// The wire of this table. Bump it whenever `snapshots/side.snap` changes.
pub const SIDE_REVISION: u16 = 1;

/// [`SideInfo::caps`] bits.
pub mod caps {
    /// `SetAgents` is served.
    pub const AGENTS: u32 = 1 << 0;
    /// `SetUsage` is served.
    pub const USAGE: u32 = 1 << 1;
    /// `GetHalvesDiag` reports the halves.
    pub const HALVES_DIAG: u32 = 1 << 2;
}

/// What `GetSideInfo` reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
pub struct SideInfo {
    pub revision: u16,
    pub caps: u32,
}

/// Agent counts by state, from the host's agent runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
pub struct AgentSummary {
    pub working: u8,
    pub idle: u8,
    pub blocked: u8,
    pub done: u8,
    pub unknown: u8,
}

/// Rate-limit use in percent; `None` for a window the host could not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
pub struct UsageSummary {
    pub five_hour: Option<u8>,
    pub seven_day: Option<u8>,
    /// The host has not refreshed the reading in a while.
    pub stale: bool,
}

/// Wake-ups and awake time of a peripheral half since it booted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
pub struct SleepStats {
    pub wakes: u16,
    pub awake_min: u16,
    pub uptime_min: u16,
}

/// How busy a half's pointing sensor loop has been: wrapping counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
pub struct PointingStats {
    pub wakes: u32,
    pub reads: u32,
}

/// A peripheral half's power diagnostics, field for field what it sends the
/// central. Counters count since boot and wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
pub struct PowerDiag {
    pub version: u8,
    pub flags: u8,
    pub battery_mv: u16,
    pub reset_reason: u32,
    pub boot_count: u16,
    pub uptime_ms: u32,
    pub cpu_active_ms: u32,
    pub key_presses: u32,
    pub rssi: i8,
    pub last_disconnect_reason: u8,
    pub disconnects: u16,
    pub conn_interval_us: u32,
    pub peripheral_latency: u16,
    pub supervision_timeout_ms: u16,
    pub param_updates: u16,
    pub split_tx: u32,
    pub split_rx: u32,
}

/// One half's diagnostics; `None` where the half has not reported, or for the
/// half that is the central itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
pub struct HalfDiag {
    pub sleep: Option<SleepStats>,
    pub pointing: Option<PointingStats>,
    pub power: Option<PowerDiag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
pub struct HalvesDiag {
    pub left: HalfDiag,
    pub right: HalfDiag,
}

endpoints! {
    // Personal (0x7EFx).
    SetAgents = 0x7EF0: AgentSummary => ();
    SetUsage = 0x7EF1: UsageSummary => ();
    GetHalvesDiag = 0x7EF8: () => HalvesDiag;
    GetSideInfo = 0x7EFF: () => SideInfo;
}

#[cfg(not(feature = "host"))]
const _: () = core::assert!(
    super::RYNK_MAX_PAYLOAD_SIZE >= MAX_ENDPOINT_PAYLOAD,
    "rynk_buffer_size is too small to hold the largest personal rynk frame; increase it"
);

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::vec::Vec;

    use super::*;
    use crate::protocol::rynk::RynkHeader;
    use crate::protocol::rynk::tests::snapshot::{assert_snapshot, format_value_snapshot};
    use crate::protocol::rynk::tests::{assert_max_size_bound, round_trip};

    #[test]
    fn commands_stay_in_the_personal_range() {
        for row in ENDPOINT_META {
            assert!((0x7EF0..=0x7EFF).contains(&row.cmd), "{} = {:#06x}", row.name, row.cmd);
        }
    }

    fn power() -> PowerDiag {
        PowerDiag {
            version: 1,
            flags: 0x21,
            battery_mv: 3900,
            reset_reason: 0x0001_0004,
            boot_count: 3,
            uptime_ms: 600_000,
            cpu_active_ms: 12_000,
            key_presses: 512,
            rssi: -60,
            last_disconnect_reason: 0x08,
            disconnects: 2,
            conn_interval_us: 7_500,
            peripheral_latency: 0,
            supervision_timeout_ms: 4_000,
            param_updates: 5,
            split_tx: 100_000,
            split_rx: 90_000,
        }
    }

    fn halves() -> HalvesDiag {
        HalvesDiag {
            left: HalfDiag {
                sleep: Some(SleepStats {
                    wakes: 7,
                    awake_min: 30,
                    uptime_min: 600,
                }),
                pointing: Some(PointingStats {
                    wakes: 70_000,
                    reads: 1_000_000,
                }),
                power: Some(power()),
            },
            right: HalfDiag {
                sleep: None,
                pointing: None,
                power: None,
            },
        }
    }

    #[test]
    fn every_payload_round_trips_within_its_max_size() {
        assert_max_size_bound(&round_trip(&halves()));
        let full = HalfDiag {
            sleep: halves().left.sleep,
            pointing: halves().left.pointing,
            power: Some(power()),
        };
        assert_max_size_bound(&round_trip(&HalvesDiag { left: full, right: full }));
        assert_max_size_bound(&round_trip(&UsageSummary {
            five_hour: Some(100),
            seven_day: None,
            stale: true,
        }));
    }

    fn encode<T: Serialize>(value: &T) -> Vec<u8> {
        let mut buf = [0u8; 512];
        postcard::to_slice(value, &mut buf).unwrap().to_vec()
    }

    fn frame<T: Serialize>(cmd: Cmd, value: &T) -> Vec<u8> {
        let mut buf = [0u8; 512];
        let n = crate::protocol::rynk::encode_frame(&mut buf, RynkHeader { cmd, seq: 1 }, value).unwrap();
        buf[..n].to_vec()
    }

    fn ok<T>(value: T) -> Result<T, RynkError> {
        Ok(value)
    }

    /// Requests and `Ok` replies (SEQ 1) of every personal command, plus the
    /// payload exemplars: the golden bytes a host checks its own copy against.
    #[test]
    fn side_wire_locked() {
        let agents = AgentSummary {
            working: 2,
            idle: 3,
            blocked: 1,
            done: 4,
            unknown: 0,
        };
        let usage = UsageSummary {
            five_hour: Some(42),
            seven_day: None,
            stale: false,
        };
        let info = SideInfo {
            revision: SIDE_REVISION,
            caps: caps::AGENTS | caps::USAGE | caps::HALVES_DIAG,
        };
        let entries: Vec<(&str, Vec<u8>)> = alloc::vec![
            ("AgentSummary", encode(&agents)),
            ("UsageSummary", encode(&usage)),
            ("HalvesDiag", encode(&halves())),
            ("SideInfo", encode(&info)),
            ("SetAgents request", frame(Cmd::SetAgents, &agents)),
            ("SetAgents reply", frame(Cmd::SetAgents, &ok(()))),
            ("SetUsage request", frame(Cmd::SetUsage, &usage)),
            ("SetUsage reply", frame(Cmd::SetUsage, &ok(()))),
            ("GetHalvesDiag request", frame(Cmd::GetHalvesDiag, &())),
            ("GetHalvesDiag reply", frame(Cmd::GetHalvesDiag, &ok(halves()))),
            ("GetSideInfo request", frame(Cmd::GetSideInfo, &())),
            ("GetSideInfo reply", frame(Cmd::GetSideInfo, &ok(info))),
        ];
        let view: Vec<(&str, &[u8])> = entries.iter().map(|(l, b)| (*l, b.as_slice())).collect();
        let actual = format_value_snapshot(
            "snapshots/side.snap",
            "Personal Qube wire snapshot",
            "# Payload exemplars and full frames (SEQ 1) of the personal commands. A diff\n\
             # here means their wire changed: bump SIDE_REVISION and regenerate with:",
            // Rides on the hint's `--features host`: the table needs `qube_side` too.
            "--features qube_side side_wire",
            &view,
        );
        assert_snapshot("snapshots/side.snap", actual);
    }
}
