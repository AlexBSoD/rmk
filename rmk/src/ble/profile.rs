//! Manage BLE profiles and bonding information

#[cfg(feature = "_ble")]
use bt_hci::{cmd::le::LeSetPhy, controller::ControllerCmdAsync};
use embassy_futures::select::{Either, select, select_array};
use embassy_sync::signal::Signal;
use embassy_sync::watch::Watch;
use trouble_host::prelude::*;
use trouble_host::{BondInformation, LongTermKey};

use super::ble_server::CCCD_TABLE_SIZE;
use crate::NUM_BLE_PROFILE;
use crate::channel::BLE_PROFILE_CHANNEL;
use crate::state::{current_profile, set_ble_bonded, set_ble_profile};
#[cfg(feature = "storage")]
use crate::storage::{StorageItem, StorageKey, StorageValue, read, store};

/// The dedicated dongle bond slot: the slot number after the normal profiles,
/// so pairing a dongle never touches a host bond. Selected only via the
/// `SwitchToDongle` key, never by profile cycling.
#[cfg(feature = "dongle")]
pub(crate) const DONGLE_PROFILE: u8 = NUM_BLE_PROFILE as u8;

/// Bond slots kept by the profile manager: the host profiles, plus the
/// dedicated dongle slot on `dongle` builds.
pub(crate) const BOND_SLOTS: usize = NUM_BLE_PROFILE + cfg!(feature = "dongle") as usize;

/// BLE profile info
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ProfileInfo {
    pub(crate) slot_num: u8,
    pub(crate) removed: bool,
    pub(crate) info: BondInformation,
    /// Raw bytes of the trouble-host `ClientAttTable` for this peer.
    /// Reconstructed via `ClientAttTableView::try_from_raw` when applied to the stack.
    pub(crate) cccd_table: heapless::Vec<u8, CCCD_TABLE_SIZE>,
}

/// Returns the maximum number of bytes required to encode T.
pub const fn varint_max<T: Sized>() -> usize {
    const BITS_PER_BYTE: usize = 8;
    const BITS_PER_VARINT_BYTE: usize = 7;

    // How many data bits do we need for this type?
    let bits = core::mem::size_of::<T>() * BITS_PER_BYTE;

    // We add (BITS_PER_VARINT_BYTE - 1), to ensure any integer divisions
    // with a remainder will always add exactly one full byte, but
    // an evenly divided number of bits will be the same
    let roundup_bits = bits + (BITS_PER_VARINT_BYTE - 1);

    // Apply division, using normal "round down" integer division
    roundup_bits / BITS_PER_VARINT_BYTE
}

// Manual MaxSize implementation
impl postcard::experimental::max_size::MaxSize for ProfileInfo {
    const POSTCARD_MAX_SIZE: usize = varint_max::<Self>();
}

impl Default for ProfileInfo {
    fn default() -> Self {
        Self {
            slot_num: 0,
            removed: false,
            info: BondInformation::new(
                Identity {
                    addr: Address::default(),
                    irk: None,
                },
                LongTermKey(0),
                SecurityLevel::NoEncryption,
                false,
            ),
            cccd_table: heapless::Vec::new(),
        }
    }
}

/// BLE profile switch action
#[derive(Debug)]
pub(crate) enum BleProfileAction {
    Switch(u8),
    Previous,
    Next,
    ClearBond,
    /// Clear the bond for an explicit slot, regardless of which slot is
    /// currently active. Rynk's `Cmd::ClearBleProfile` issues this so a host
    /// tool can wipe any bond without first switching to it.
    ClearSlot(u8),
}

/// Manage BLE profiles and bonding information
///
/// ProfileManager is responsible for:
/// 1. Managing multiple BLE profiles, allowing users to switch between multiple devices
/// 2. Storing and loading bonding information for each profile
/// 3. Updating the bonding information of the active profile to the BLE stack
/// 4. Handling profile switch, clear, and save operations
///
/// `SLOTS` sizes the cache to the role: [`BOND_SLOTS`] for a keyboard, a single
/// slot for a dongle.
#[cfg(feature = "_ble")]
pub(crate) struct ProfileManager<
    'b,
    's,
    C: Controller + ControllerCmdAsync<LeSetPhy>,
    P: PacketPool,
    const SLOTS: usize,
> where
    's: 'b,
{
    profiles: [Watch<crate::RawMutex, ProfileInfo, 1>; SLOTS],
    updated_active_profile: Signal<crate::RawMutex, ()>,
    /// BLE stack
    stack: &'b Stack<'s, C, P>,
}

#[cfg(feature = "_ble")]
impl<'b, 's, C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool, const SLOTS: usize>
    ProfileManager<'b, 's, C, P, SLOTS>
where
    's: 'b,
{
    /// Create a new profile manager
    pub(crate) fn new(stack: &'b Stack<'s, C, P>) -> Self {
        Self {
            profiles: [const { Watch::new() }; SLOTS],
            updated_active_profile: Signal::new(),
            stack,
        }
    }

    /// Load stored bonding information
    #[cfg(feature = "storage")]
    pub(crate) async fn load_bonded_devices(&mut self) {
        for (slot, state) in self.profiles.iter_mut().enumerate() {
            *state = match read(StorageKey::BondInfo(slot as u8)).await {
                Ok(Some(StorageValue::BondInfo(info))) if !info.removed => Watch::new_with(info),
                _ => Watch::new(),
            };
        }

        let profile =
            if let Ok(Some(StorageValue::ActiveBleProfile(profile))) = read(StorageKey::ActiveBleProfile).await {
                debug!("Loaded active profile: {}", profile);
                profile
            } else {
                debug!("Loaded default active profile",);
                0
            };
        set_ble_profile(profile, self.bond_info(profile).is_some());
    }

    /// Read the live bond stored in `profile`.
    pub(crate) fn bond_info(&self, profile: u8) -> Option<ProfileInfo> {
        self.profiles.get(profile as usize)?.try_get_and(|info| !info.removed)
    }

    /// Check if the `identity` is the bonded dongle's identity.
    ///
    /// This function is used when searching for BLE host,
    /// the connected dongle(in the DONGLE_PROFILE) should be excluded.
    #[cfg(feature = "dongle")]
    pub(crate) fn is_bonded_dongle(&self, identity: &Identity) -> bool {
        self.bond_info(DONGLE_PROFILE)
            .is_some_and(|info| info.info.identity.match_identity(identity))
    }

    /// Update bonding information in the stack according to the current active profile
    ///
    /// Also republishes `BleStatus::bonded`. Every change to `profiles` that can
    /// alter bond presence is followed by this call, so the flag is derived here once
    /// instead of at each mutation site.
    pub(crate) fn update_stack_bonds(&self) {
        // Drain one at a time rather than collecting: the stack holds bonds this
        // manager has no slot for — a fresh pairing lands there before we prune —
        // and a `heapless::Vec` collect panics on the overflow.
        while let Some(identity) = self
            .stack
            .with_bond_information(|bonds| bonds.first().map(|b| b.identity))
        {
            if let Err(e) = self.stack.remove_bond_information(identity) {
                debug!("Remove bond info error: {:?}", e);
                break; // a bond that won't come off would spin here forever
            }
        }

        let active = self.bond_info(current_profile());
        set_ble_bonded(active.is_some());

        if let Some(info) = active {
            debug!("Add bond info of profile {}: {:?}", info.slot_num, info);
            if let Err(e) = self.stack.add_bond_information(info.info) {
                debug!("Add bond info error: {:?}", e);
            }
        }
    }

    /// Set a slot's bond and initial CCCD state.
    pub(crate) fn set_bond(&self, info: ProfileInfo) {
        let Some(state) = self.profiles.get(info.slot_num as usize) else {
            return;
        };
        let old = state.try_get();
        if old.as_ref() == Some(&info) {
            return;
        }
        let bond_changed = old.as_ref().is_none_or(|old| old.removed || old.info != info.info);
        state.sender().send(info);
        if bond_changed {
            self.update_stack_bonds();
        }
    }

    pub(crate) fn update_profile_cccd_table(&self, slot: u8, table: heapless::Vec<u8, CCCD_TABLE_SIZE>) {
        if let Some(state) = self.profiles.get(slot as usize) {
            state.sender().send_if_modified(|info| {
                if let Some(info) = info
                    && !info.removed
                    && info.cccd_table != table
                {
                    info.cccd_table = table.clone();
                    return true;
                }
                false
            });
        }
    }

    /// One persistence receiver per slot, retained across connection changes.
    pub(crate) async fn run(&self) -> ! {
        let mut receivers = self
            .profiles
            .each_ref()
            .map(|state| state.receiver().expect("one profile saver"));
        loop {
            let item = match select(
                select_array(receivers.each_mut().map(|receiver| receiver.changed())),
                self.updated_active_profile.wait(),
            )
            .await
            {
                Either::First((info, _)) => StorageItem::BondInfo(info),
                Either::Second(()) => StorageItem::ActiveBleProfile(current_profile()),
            };
            if store(item).await.is_err() {
                error!("Failed to save BLE profile");
            }
        }
    }

    /// Clear bonding information of the specified slot.
    pub(crate) fn clear_bond(&self, slot: u8) {
        if let Some(state) = self.profiles.get(slot as usize) {
            state.sender().send(ProfileInfo {
                slot_num: slot,
                removed: true,
                ..Default::default()
            });
            self.update_stack_bonds();
        }
    }

    /// Apply one profile action, then let the connection loop restart.
    pub(crate) async fn update_profile(&self) {
        loop {
            let action = BLE_PROFILE_CHANNEL.receive().await;
            let current = current_profile();
            let profile = match action {
                BleProfileAction::Switch(profile) if profile == current => continue,
                BleProfileAction::Switch(profile) => profile,
                BleProfileAction::Previous => {
                    if current == 0 {
                        NUM_BLE_PROFILE as u8 - 1
                    } else {
                        current - 1
                    }
                }
                BleProfileAction::Next => {
                    let next = current + 1;
                    if next >= NUM_BLE_PROFILE as u8 { 0 } else { next }
                }
                BleProfileAction::ClearBond => {
                    self.clear_bond(current);
                    return;
                }
                BleProfileAction::ClearSlot(slot) => {
                    self.clear_bond(slot);
                    return;
                }
            };
            if profile != current {
                set_ble_profile(profile, self.bond_info(profile).is_some());
                self.update_stack_bonds();
                self.updated_active_profile.signal(());
                info!("Switched to BLE profile: {}", profile);
            }
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use bt_hci::controller::ExternalController;
    use bt_hci::transport::{PacketToController, PacketToHost, Transport};

    use super::{DefaultPacketPool, HostResources, LongTermKey, ProfileInfo, ProfileManager};
    use crate::test_support::test_block_on;

    struct TestTransport;

    impl embedded_io_async::ErrorType for TestTransport {
        type Error = bt_hci::ReadHciError<core::convert::Infallible>;
    }

    impl Transport for TestTransport {
        async fn read<'a, P: PacketToHost<'a>>(&self, _: &'a mut [u8]) -> Result<P, Self::Error> {
            panic!("unexpected controller read");
        }

        async fn write<P: PacketToController>(&self, _: &P) -> Result<(), Self::Error> {
            panic!("unexpected controller write");
        }
    }

    type TestManager<'b, 's, const SLOTS: usize> =
        ProfileManager<'b, 's, ExternalController<TestTransport, 1>, DefaultPacketPool, SLOTS>;

    fn with_manager<const SLOTS: usize>(test: impl FnOnce(&mut TestManager<'_, '_, SLOTS>)) {
        let mut resources = HostResources::<DefaultPacketPool, 1, 1>::new();
        let stack = trouble_host::new(ExternalController::<_, 1>::new(TestTransport), &mut resources).build();
        let mut manager = ProfileManager::new(&stack);
        test(&mut manager);
    }

    #[test]
    fn cleared_profile_can_be_paired_again_with_same_bond_information() {
        with_manager::<1>(|manager| {
            let profile_info = ProfileInfo::default();
            manager.set_bond(profile_info.clone());
            manager.set_bond(profile_info.clone());

            manager.clear_bond(0);
            manager.update_profile_cccd_table(0, heapless::Vec::from_slice(&[1]).unwrap());
            assert!(manager.bond_info(0).is_none());
            manager.set_bond(profile_info);
            assert!(!manager.profiles[0].try_get().unwrap().removed);
            assert_eq!(manager.profiles.iter().filter(|p| p.try_get().is_some()).count(), 1);
        });
    }

    #[test]
    fn out_of_range_slot_does_not_replace_an_existing_profile() {
        with_manager::<1>(|manager| {
            manager.set_bond(ProfileInfo::default());
            let other_slot = ProfileInfo {
                slot_num: 1,
                ..Default::default()
            };
            manager.set_bond(other_slot);
            assert_eq!(manager.profiles.iter().filter(|p| p.try_get().is_some()).count(), 1);
            assert_eq!(manager.profiles[0].try_get().unwrap().slot_num, 0);
        });
    }

    #[test]
    fn cleared_slot_has_no_bond_info() {
        with_manager::<1>(|manager| {
            manager.set_bond(ProfileInfo::default());
            assert!(manager.bond_info(0).is_some());
            manager.clear_bond(0);
            assert!(manager.bond_info(0).is_none());
        });
    }

    #[test]
    fn bonding_one_slot_leaves_the_others_unbonded() {
        with_manager::<2>(|manager| {
            manager.set_bond(ProfileInfo::default());
            assert!(manager.bond_info(0).is_some());
            assert!(manager.bond_info(1).is_none());
        });
    }

    #[test]
    fn re_pairing_a_slot_with_different_bond_information_replaces_it() {
        with_manager::<1>(|manager| {
            manager.set_bond(ProfileInfo::default());
            let mut new_host = ProfileInfo::default();
            new_host.info.ltk = LongTermKey(1);
            manager.set_bond(new_host);
            assert_eq!(manager.profiles[0].try_get().unwrap().info.ltk, LongTermKey(1));
            assert_eq!(manager.profiles.iter().filter(|p| p.try_get().is_some()).count(), 1);
        });
    }

    #[test]
    fn updates_during_storage_wait_are_saved_from_latest_cache() {
        use core::future::Future;
        use core::pin::pin;
        use core::task::{Context, Waker};

        use embassy_futures::select::{Either3, select3};

        use crate::config::StorageConfig;
        use crate::core_traits::Runnable;
        use crate::storage::{Storage, StorageKey, StorageValue, async_flash_wrapper, read};
        use crate::test_support::{InMemoryFlash, clear_flash_channel};

        with_manager::<2>(|manager| {
            clear_flash_channel();
            for (slot, key) in [(0, 1), (1, 2)] {
                let mut info = ProfileInfo {
                    slot_num: slot,
                    ..Default::default()
                };
                info.info.ltk = LongTermKey(key);
                manager.set_bond(info);
            }
            let mut saver = pin!(manager.run());
            {
                let mut read = pin!(read(StorageKey::StorageConfig));
                let mut cx = Context::from_waker(Waker::noop());
                assert!(read.as_mut().poll(&mut cx).is_pending());
                assert!(saver.as_mut().poll(&mut cx).is_pending());
            }
            for slot in 0..2 {
                manager.update_profile_cccd_table(slot, heapless::Vec::from_slice(&[slot + 3]).unwrap());
            }
            // Connection-level cancellation does not drop the independent saver.
            {
                let mut update = pin!(manager.update_profile());
                let mut cx = Context::from_waker(Waker::noop());
                assert!(update.as_mut().poll(&mut cx).is_pending());
            }
            test_block_on(async {
                let flash = async_flash_wrapper(InMemoryFlash::<16_384, 4_096, 1>::new());
                let mut storage = Box::pin(Storage::new(flash, &StorageConfig::default())).await;
                let verify = async {
                    for slot in 0..2 {
                        loop {
                            if let Some(StorageValue::BondInfo(info)) = read(StorageKey::BondInfo(slot)).await.unwrap()
                                && info.cccd_table.as_slice() == [slot + 3]
                            {
                                assert_eq!(info.info.ltk, LongTermKey((slot + 1).into()));
                                break;
                            }
                        }
                    }
                };
                assert!(matches!(
                    select3(saver.as_mut(), storage.run(), verify).await,
                    Either3::Third(())
                ));
            });
        });
    }

    #[test]
    fn clear_during_storage_wait_persists_tombstone_last() {
        use core::future::Future;
        use core::pin::pin;
        use core::task::{Context, Waker};

        use embassy_futures::select::{Either3, select3};

        use crate::config::StorageConfig;
        use crate::core_traits::Runnable;
        use crate::storage::{Storage, StorageKey, StorageValue, async_flash_wrapper, read};
        use crate::test_support::{InMemoryFlash, clear_flash_channel};

        with_manager::<1>(|manager| {
            clear_flash_channel();
            manager.set_bond(ProfileInfo::default());
            let mut saver = pin!(manager.run());
            {
                let mut read = pin!(read(StorageKey::StorageConfig));
                let mut cx = Context::from_waker(Waker::noop());
                assert!(read.as_mut().poll(&mut cx).is_pending());
                assert!(saver.as_mut().poll(&mut cx).is_pending());
            }
            manager.clear_bond(0);
            manager.update_profile_cccd_table(0, heapless::Vec::from_slice(&[1]).unwrap());
            test_block_on(async {
                let flash = async_flash_wrapper(InMemoryFlash::<16_384, 4_096, 1>::new());
                let mut storage = Box::pin(Storage::new(flash, &StorageConfig::default())).await;
                let verify = async {
                    loop {
                        if let Some(StorageValue::BondInfo(info)) = read(StorageKey::BondInfo(0)).await.unwrap()
                            && info.removed
                        {
                            break;
                        }
                    }
                };
                assert!(matches!(
                    select3(saver.as_mut(), storage.run(), verify).await,
                    Either3::Third(())
                ));
            });
            assert!(manager.bond_info(0).is_none());
        });
    }

    #[test]
    fn profile_and_active_notifications_are_all_persisted() {
        use embassy_futures::select::{Either3, select3};

        use crate::config::StorageConfig;
        use crate::core_traits::Runnable;
        use crate::storage::{Storage, StorageKey, StorageValue, async_flash_wrapper, read};
        use crate::test_support::{InMemoryFlash, clear_flash_channel};

        with_manager::<2>(|manager| {
            clear_flash_channel();
            manager.clear_bond(0);
            manager.set_bond(ProfileInfo {
                slot_num: 1,
                ..Default::default()
            });
            test_block_on(async {
                super::BLE_PROFILE_CHANNEL
                    .try_send(super::BleProfileAction::Switch(1))
                    .unwrap();
                manager.update_profile().await;
                let flash = async_flash_wrapper(InMemoryFlash::<16_384, 4_096, 1>::new());
                let mut storage = Box::pin(Storage::new(flash, &StorageConfig::default())).await;
                let verify = async {
                    for slot in 0..2 {
                        loop {
                            if let Some(StorageValue::BondInfo(info)) = read(StorageKey::BondInfo(slot)).await.unwrap()
                            {
                                assert_eq!(info.removed, slot == 0);
                                break;
                            }
                        }
                    }
                    loop {
                        if let Some(StorageValue::ActiveBleProfile(active)) =
                            read(StorageKey::ActiveBleProfile).await.unwrap()
                        {
                            assert_eq!(active, 1);
                            break;
                        }
                    }
                };
                assert!(matches!(
                    select3(manager.run(), storage.run(), verify).await,
                    Either3::Third(())
                ));
            });
        });
    }

    #[test]
    fn a_failed_save_does_not_prevent_saving_later_updates() {
        use core::pin::pin;

        use embassy_futures::select::{Either3, select3};
        use embassy_time::Timer;

        use crate::config::StorageConfig;
        use crate::core_traits::Runnable;
        use crate::storage::{Storage, StorageKey, StorageValue, async_flash_wrapper, read};
        use crate::test_support::{InMemoryFlash, clear_flash_channel};
        with_manager::<1>(|manager| {
            clear_flash_channel();
            test_block_on(async {
                let flash = InMemoryFlash::<16_384, 4_096, 1>::new();
                let control = flash.clone();
                let mut storage = Box::pin(Storage::new(async_flash_wrapper(flash), &StorageConfig::default())).await;
                control.fail_writes(true);
                manager.set_bond(ProfileInfo::default());
                let mut saver = pin!(manager.run());
                select3(saver.as_mut(), storage.run(), Timer::after_millis(10)).await;
                assert!(storage.fetch(StorageKey::BondInfo(0)).await.unwrap().is_none());
                control.fail_writes(false);
                manager.update_profile_cccd_table(0, heapless::Vec::from_slice(&[1]).unwrap());
                let verify = async {
                    loop {
                        if let Some(StorageValue::BondInfo(info)) = read(StorageKey::BondInfo(0)).await.unwrap() {
                            assert_eq!(info.cccd_table.as_slice(), &[1]);
                            break;
                        }
                    }
                };
                assert!(matches!(
                    select3(saver.as_mut(), storage.run(), verify).await,
                    Either3::Third(())
                ));
            });
        });
    }

    #[test]
    fn switching_does_not_wait_for_pending_flash() {
        use core::future::Future;
        use core::pin::pin;
        use core::task::{Context, Waker};

        use crate::storage::{StorageKey, read};
        use crate::test_support::clear_flash_channel;
        with_manager::<2>(|manager| {
            clear_flash_channel();
            manager.set_bond(ProfileInfo::default());
            let mut blocker = pin!(read(StorageKey::StorageConfig));
            let mut saver = pin!(manager.run());
            let mut cx = Context::from_waker(Waker::noop());
            assert!(blocker.as_mut().poll(&mut cx).is_pending());
            assert!(saver.as_mut().poll(&mut cx).is_pending());
            super::BLE_PROFILE_CHANNEL
                .try_send(super::BleProfileAction::Switch(1))
                .unwrap();
            {
                let mut action = pin!(manager.update_profile());
                assert!(action.as_mut().poll(&mut cx).is_ready());
            }
            assert_eq!(crate::state::current_profile(), 1);
            super::BLE_PROFILE_CHANNEL
                .try_send(super::BleProfileAction::ClearBond)
                .unwrap();
            {
                let mut action = pin!(manager.update_profile());
                assert!(action.as_mut().poll(&mut cx).is_ready());
            }
            assert!(manager.profiles[1].try_get().unwrap().removed);
            assert!(saver.as_mut().poll(&mut cx).is_pending());
        });
    }

    #[test]
    fn loading_profiles_does_not_schedule_redundant_saves() {
        use embassy_futures::select::{select, select3};
        use embassy_time::Timer;

        use crate::config::StorageConfig;
        use crate::core_traits::Runnable;
        use crate::storage::{Storage, StorageItem, async_flash_wrapper, store};
        use crate::test_support::{InMemoryFlash, clear_flash_channel};
        with_manager::<2>(|manager| {
            clear_flash_channel();
            test_block_on(async {
                let flash = InMemoryFlash::<16_384, 4_096, 1>::new();
                let control = flash.clone();
                let mut storage = Box::pin(Storage::new(async_flash_wrapper(flash), &StorageConfig::default())).await;
                select(store(StorageItem::BondInfo(ProfileInfo::default())), storage.run()).await;
                select(manager.load_bonded_devices(), storage.run()).await;
                assert!(manager.bond_info(0).is_some());
                let before = control.writes();
                select3(manager.run(), storage.run(), Timer::after_millis(10)).await;
                assert_eq!(control.writes(), before);
            });
        });
    }

    #[test]
    fn clear_and_repair_during_write_survives_reload() {
        use core::future::Future;
        use core::pin::pin;
        use core::task::{Context, Waker};

        use embassy_futures::select::{Either3, select, select3};

        use crate::config::StorageConfig;
        use crate::core_traits::Runnable;
        use crate::storage::{Storage, StorageKey, StorageValue, async_flash_wrapper, read};
        use crate::test_support::{InMemoryFlash, clear_flash_channel};
        with_manager::<1>(|manager| {
            clear_flash_channel();
            test_block_on(async {
                let flash = InMemoryFlash::<16_384, 4_096, 1>::new();
                let reboot = flash.clone();
                let mut storage = Box::pin(Storage::new(async_flash_wrapper(flash), &StorageConfig::default())).await;
                manager.set_bond(ProfileInfo::default());
                {
                    let mut saver = pin!(manager.run());
                    {
                        let mut blocker = pin!(read(StorageKey::StorageConfig));
                        let mut cx = Context::from_waker(Waker::noop());
                        assert!(blocker.as_mut().poll(&mut cx).is_pending());
                        assert!(saver.as_mut().poll(&mut cx).is_pending());
                        manager.clear_bond(0);
                        let mut replacement = ProfileInfo::default();
                        replacement.info.ltk = LongTermKey(42);
                        manager.set_bond(replacement);
                        manager.update_profile_cccd_table(0, heapless::Vec::from_slice(&[1, 2]).unwrap());
                    }
                    let verify = async {
                        loop {
                            if let Some(StorageValue::BondInfo(info)) = read(StorageKey::BondInfo(0)).await.unwrap()
                                && info.info.ltk == LongTermKey(42)
                            {
                                assert!(!info.removed);
                                assert_eq!(info.cccd_table.as_slice(), &[1, 2]);
                                break;
                            }
                        }
                    };
                    assert!(matches!(
                        select3(saver.as_mut(), storage.run(), verify).await,
                        Either3::Third(())
                    ));
                }
                drop(storage);
                let mut storage = Box::pin(Storage::new(async_flash_wrapper(reboot), &StorageConfig::default())).await;
                select(manager.load_bonded_devices(), storage.run()).await;
                let restored = manager.bond_info(0).unwrap();
                assert_eq!(restored.info.ltk, LongTermKey(42));
                assert_eq!(restored.cccd_table.as_slice(), &[1, 2]);
            });
        });
    }

    #[test]
    fn unchanged_cccd_does_not_write_flash() {
        use core::pin::pin;

        use embassy_futures::select::select3;
        use embassy_time::Timer;

        use crate::config::StorageConfig;
        use crate::core_traits::Runnable;
        use crate::storage::{Storage, async_flash_wrapper};
        use crate::test_support::{InMemoryFlash, clear_flash_channel};
        with_manager::<1>(|manager| {
            clear_flash_channel();
            test_block_on(async {
                let flash = InMemoryFlash::<16_384, 4_096, 1>::new();
                let control = flash.clone();
                let mut storage = Box::pin(Storage::new(async_flash_wrapper(flash), &StorageConfig::default())).await;
                manager.set_bond(ProfileInfo {
                    cccd_table: heapless::Vec::from_slice(&[1]).unwrap(),
                    ..Default::default()
                });
                let mut saver = pin!(manager.run());
                select3(saver.as_mut(), storage.run(), Timer::after_millis(2)).await;
                let before = control.writes();
                for _ in 0..10 {
                    manager.update_profile_cccd_table(0, heapless::Vec::from_slice(&[1]).unwrap());
                }
                select3(saver.as_mut(), storage.run(), Timer::after_millis(2)).await;
                assert_eq!(control.writes(), before);
                manager.update_profile_cccd_table(0, heapless::Vec::from_slice(&[0]).unwrap());
                select3(saver.as_mut(), storage.run(), Timer::after_millis(2)).await;
                assert!(control.writes() > before);
            });
        });
    }
}
