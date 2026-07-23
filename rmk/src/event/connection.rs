//! Connection related events
//!
//! Single event published whenever the `ConnectionStatus` changes

use rmk_macro::event;
pub use rmk_types::connection::{ConnectionStatus, ConnectionType};

/// Host advertising mode used by runtime indicators.
///
/// This is intentionally separate from [`rmk_types::ble::BleState`] so the
/// public connection-status wire format remains stable.
#[cfg(feature = "_ble")]
#[repr(u8)]
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, postcard::experimental::max_size::MaxSize,
)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BleAdvertisingMode {
    /// No bond exists for the active profile; advertise for a new host.
    Pairing,
    /// A bond exists for the active profile; reconnect to that host.
    Reconnecting,
}

/// Runtime host-advertising mode changed event.
#[cfg(feature = "_ble")]
#[event(channel_size = crate::BLE_ADVERTISING_MODE_EVENT_CHANNEL_SIZE, pubs = crate::BLE_ADVERTISING_MODE_EVENT_PUB_SIZE, subs = crate::BLE_ADVERTISING_MODE_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct BleAdvertisingModeEvent(pub BleAdvertisingMode);

#[cfg(feature = "_ble")]
impl_payload_wrapper!(BleAdvertisingModeEvent, BleAdvertisingMode);

/// `ConnectionStatus` changed event. Fires from `state::update_status` whenever
/// the connection status updates
#[event(channel_size = crate::CONNECTION_STATUS_CHANGE_EVENT_CHANNEL_SIZE, pubs = crate::CONNECTION_STATUS_CHANGE_EVENT_PUB_SIZE, subs = crate::CONNECTION_STATUS_CHANGE_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ConnectionStatusChangeEvent(pub ConnectionStatus);

impl_payload_wrapper!(ConnectionStatusChangeEvent, ConnectionStatus);
