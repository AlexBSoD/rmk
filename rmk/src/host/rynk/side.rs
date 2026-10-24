//! Personal Qube commands (`rmk_types::protocol::rynk::side`): the host-agent
//! window and the halves' diagnostics. No release carries them.

#[cfg(all(feature = "split", feature = "_ble"))]
use rmk_types::protocol::rynk::side::{PointingStats, PowerDiag, SleepStats};
use rmk_types::protocol::rynk::side::{
    AgentSummary, GetHalvesDiag, GetSideInfo, HalfDiag, HalvesDiag, SIDE_REVISION, SetAgents, SetUsage, SideInfo,
    UsageSummary, caps,
};
use rmk_types::protocol::rynk::{Cmd, RynkError, RynkMessage};

use super::RynkService;
use super::handlers::{Handle, serve};
use crate::host_data;

/// A half that has not reported, or the central itself.
const NO_DIAG: HalfDiag = HalfDiag {
    sleep: None,
    pointing: None,
    power: None,
};

impl RynkService<'_> {
    /// Serve `msg` when it is a personal command, `None` otherwise.
    pub(super) async fn dispatch_side(&self, msg: &mut RynkMessage<'_>) -> Option<Result<(), RynkError>> {
        Some(match msg.header().cmd {
            Cmd::SetAgents => serve::<SetAgents, _>(self, msg).await,
            Cmd::SetUsage => serve::<SetUsage, _>(self, msg).await,
            Cmd::GetHalvesDiag => serve::<GetHalvesDiag, _>(self, msg).await,
            Cmd::GetSideInfo => serve::<GetSideInfo, _>(self, msg).await,
            _ => return None,
        })
    }
}

impl Handle<SetAgents> for RynkService<'_> {
    async fn handle(&self, agents: AgentSummary) -> Result<(), RynkError> {
        host_data::update_agents(host_data::AgentSummary {
            working: agents.working,
            idle: agents.idle,
            blocked: agents.blocked,
            done: agents.done,
            unknown: agents.unknown,
        });
        Ok(())
    }
}

impl Handle<SetUsage> for RynkService<'_> {
    async fn handle(&self, usage: UsageSummary) -> Result<(), RynkError> {
        // Whole percent: past 100 is a host bug, clamped so a bar cannot overflow.
        host_data::update_usage(host_data::UsageSummary {
            five_hour: usage.five_hour.map(|percent| percent.min(100)),
            seven_day: usage.seven_day.map(|percent| percent.min(100)),
            stale: usage.stale,
        });
        Ok(())
    }
}

impl Handle<GetHalvesDiag> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<HalvesDiag, RynkError> {
        #[cfg(all(feature = "split", feature = "_ble"))]
        {
            let (left, right) = crate::host::context::halves_for_split(
                NO_DIAG,
                peripheral_diag(0),
                peripheral_diag(1),
                crate::SPLIT_PERIPHERALS_NUM,
                crate::SPLIT_CENTRAL_IS_LEFT,
            );
            Ok(HalvesDiag { left, right })
        }
        #[cfg(not(all(feature = "split", feature = "_ble")))]
        {
            Ok(HalvesDiag {
                left: NO_DIAG,
                right: NO_DIAG,
            })
        }
    }
}

impl Handle<GetSideInfo> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<SideInfo, RynkError> {
        let halves = if cfg!(all(feature = "split", feature = "_ble")) {
            caps::HALVES_DIAG
        } else {
            0
        };
        Ok(SideInfo {
            revision: SIDE_REVISION,
            caps: caps::AGENTS | caps::USAGE | halves,
        })
    }
}

/// What peripheral `id` last reported with its battery.
#[cfg(all(feature = "split", feature = "_ble"))]
fn peripheral_diag(id: usize) -> HalfDiag {
    let sleep = crate::split::sleep_stats::peripheral_sleep_stats(id);
    HalfDiag {
        sleep: sleep.map(|s| SleepStats {
            wakes: s.wakes,
            awake_min: s.awake_min,
            uptime_min: s.uptime_min,
        }),
        // The sensor loop counters ride in the same report as the sleep ones.
        pointing: sleep.map(|s| PointingStats {
            wakes: s.pointing_wakes,
            reads: s.pointing_reads,
        }),
        power: crate::split::diag_stats::peripheral_diag_stats(id).map(|d| PowerDiag {
            version: d.version,
            flags: d.flags,
            battery_mv: d.battery_mv,
            reset_reason: d.reset_reason,
            boot_count: d.boot_count,
            uptime_ms: d.uptime_ms,
            cpu_active_ms: d.cpu_active_ms,
            key_presses: d.key_presses,
            rssi: d.rssi,
            last_disconnect_reason: d.last_disconnect_reason,
            disconnects: d.disconnects,
            conn_interval_us: d.conn_interval_us,
            peripheral_latency: d.peripheral_latency,
            supervision_timeout_ms: d.supervision_timeout_ms,
            param_updates: d.param_updates,
            split_tx: d.split_tx,
            split_rx: d.split_rx,
        }),
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::collections::VecDeque;
    use alloc::vec::Vec;

    use rmk_types::action::KeyAction;

    use super::super::tests::{ChunkRead, VecWrite, decode_frames, req, req_with};
    use super::*;
    use crate::config::{BehaviorConfig, PositionalConfig, RmkConfig};
    use crate::keymap::{KeyMap, KeymapData};
    use crate::test_support::test_block_on as block_on;

    /// Serve `requests` on one session and return each reply's payload.
    fn exchange(requests: impl IntoIterator<Item = Vec<u8>>) -> Vec<Vec<u8>> {
        let mut behavior = BehaviorConfig::default();
        let positional: PositionalConfig<1, 1> = PositionalConfig::default();
        let mut data: KeymapData<1, 1, 1, 0> = KeymapData::new([[[KeyAction::No]]]);
        let keymap = block_on(KeyMap::new(&mut data, &mut behavior, &positional));
        let service = RynkService::new(&keymap, &RmkConfig::default());
        let mut rx = ChunkRead {
            chunks: VecDeque::from_iter(requests),
        };
        let mut tx = VecWrite { captured: Vec::new() };
        block_on(service.run_session(&mut rx, &mut tx));
        decode_frames(&tx.captured)
            .into_iter()
            .map(|(_, _, payload)| payload)
            .collect()
    }

    fn reply<T: serde::de::DeserializeOwned>(payload: &[u8]) -> Result<T, RynkError> {
        postcard::from_bytes::<Result<T, RynkError>>(payload).unwrap()
    }

    /// What the host's agent runner sends lands where the Qube screen reads it.
    #[test]
    fn agents_and_usage_reach_the_screen() {
        let agents = AgentSummary {
            working: 2,
            idle: 3,
            blocked: 1,
            done: 4,
            unknown: 0,
        };
        let usage = UsageSummary {
            five_hour: Some(180),
            seven_day: None,
            stale: true,
        };
        let replies = exchange([
            req_with(Cmd::SetAgents.raw(), 1, &agents),
            req_with(Cmd::SetUsage.raw(), 2, &usage),
        ]);
        assert!(replies.iter().all(|r| reply::<()>(r) == Ok(())));

        let host = host_data::snapshot();
        assert_eq!(
            host.agents,
            Some(host_data::AgentSummary {
                working: 2,
                idle: 3,
                blocked: 1,
                done: 4,
                unknown: 0,
            })
        );
        assert_eq!(
            host.usage,
            Some(host_data::UsageSummary {
                five_hour: Some(100),
                seven_day: None,
                stale: true,
            })
        );
    }

    /// Before any half reports, both answer `None` rather than zeros.
    #[test]
    fn halves_without_a_report_answer_none() {
        let replies = exchange([req(Cmd::GetHalvesDiag.raw(), 1), req(Cmd::GetSideInfo.raw(), 2)]);
        assert_eq!(
            reply::<HalvesDiag>(&replies[0]),
            Ok(HalvesDiag {
                left: NO_DIAG,
                right: NO_DIAG,
            })
        );
        let info = reply::<SideInfo>(&replies[1]).unwrap();
        assert_eq!(info.revision, SIDE_REVISION);
        assert_eq!(info.caps & (caps::AGENTS | caps::USAGE), caps::AGENTS | caps::USAGE);
    }
}
