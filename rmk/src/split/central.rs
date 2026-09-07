#[cfg(feature = "_ble")]
use core::cell::RefCell;

#[cfg(not(feature = "_ble"))]
use embedded_io_async::{Read, Write};
#[cfg(feature = "_ble")]
use {
    bt_hci::cmd::le::{LeReadLocalSupportedFeatures, LeSetPhy, LeSetScanParams},
    bt_hci::cmd::status::ReadRssi,
    bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync},
    heapless::VecView,
    trouble_host::prelude::*,
};

#[cfg(feature = "dfu_split")]
pub use crate::split::driver::UpdatePolicy;

/// Run central's peripheral manager task.
///
/// # Arguments
/// * `id` - peripheral id
/// * `addr` - (optional) peripheral's BLE static address. This argument is enabled only for nRF BLE split now
/// * `stack` - (optional) BLE stack. This argument is enabled only for BLE split now
/// * `receiver` - (optional) serial port. This argument is enabled only for serial split now
/// * `policy` - (optional, `dfu_split` only) how to decide whether to update the peripheral's firmware
#[allow(clippy::extra_unused_lifetimes)]
pub async fn run_peripheral_manager<
    'b,
    's,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
    #[cfg(feature = "_ble")] C: Controller
        + ControllerCmdSync<LeSetScanParams>
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<ReadRssi>,
    #[cfg(not(feature = "_ble"))] S: Read + Write,
>(
    id: usize,
    #[cfg(feature = "_ble")] addr: &RefCell<VecView<Option<[u8; 6]>>>,
    #[cfg(feature = "_ble")] stack: &'b Stack<'s, C, DefaultPacketPool>,
    #[cfg(not(feature = "_ble"))] receiver: S,
    #[cfg(feature = "dfu_split")] policy: crate::split::driver::UpdatePolicy,
) where
    's: 'b,
{
    #[cfg(feature = "_ble")]
    {
        use crate::split::ble::central::{SplitLinkProfile, run_ble_peripheral_manager};
        run_ble_peripheral_manager::<C, ROW, COL, ROW_OFFSET, COL_OFFSET>(
            id,
            addr,
            stack,
            SplitLinkProfile::Keyboard,
            true,
        )
        .await;
    };

    #[cfg(not(feature = "_ble"))]
    {
        use crate::split::serial::run_serial_peripheral_manager;
        run_serial_peripheral_manager::<ROW, COL, ROW_OFFSET, COL_OFFSET, S>(
            id,
            receiver,
            #[cfg(feature = "dfu_split")]
            policy,
        )
        .await;
    }
}

/// Run a BLE split peripheral manager with an explicit connection profile and
/// the legacy 2M PHY default. Hand-written integrations can keep using this
/// source-compatible entry point; generated keyboards use the PHY-aware
/// variant below.
#[cfg(feature = "_ble")]
pub async fn run_peripheral_manager_with_profile<
    'b,
    's,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
    C: Controller
        + ControllerCmdSync<LeSetScanParams>
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<ReadRssi>,
>(
    id: usize,
    addr: &RefCell<VecView<Option<[u8; 6]>>>,
    stack: &'b Stack<'s, C, DefaultPacketPool>,
    profile: crate::split::ble::central::SplitLinkProfile,
) where
    's: 'b,
{
    run_peripheral_manager_with_profile_and_phy::<ROW, COL, ROW_OFFSET, COL_OFFSET, C>(id, addr, stack, profile, true)
        .await;
}

/// Run a BLE split peripheral manager with an explicit connection profile and
/// PHY policy. Macro-generated keyboards use this entry point so their
/// configured `use_2m_phy` value is respected.
#[cfg(feature = "_ble")]
pub async fn run_peripheral_manager_with_profile_and_phy<
    'b,
    's,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
    C: Controller
        + ControllerCmdSync<LeSetScanParams>
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<ReadRssi>,
>(
    id: usize,
    addr: &RefCell<VecView<Option<[u8; 6]>>>,
    stack: &'b Stack<'s, C, DefaultPacketPool>,
    profile: crate::split::ble::central::SplitLinkProfile,
    use_2m_phy: bool,
) where
    's: 'b,
{
    crate::split::ble::central::run_ble_peripheral_manager::<C, ROW, COL, ROW_OFFSET, COL_OFFSET>(
        id, addr, stack, profile, use_2m_phy,
    )
    .await;
}
