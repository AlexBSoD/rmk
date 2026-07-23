use bt_hci::cmd::le::LeSetPhy;
use bt_hci::controller::ControllerCmdAsync;
use embassy_futures::join::join;
use embassy_time::{Duration, Timer, with_timeout};
use rmk_types::connection::ConnectionStatus;
use trouble_host::prelude::*;

#[cfg(feature = "storage")]
use super::PeerAddress;
use crate::event::{CentralConnectedEvent, KeyboardEvent, SubscribableEvent, publish_event};
use crate::split::driver::{SplitDriverError, SplitReader, SplitWriter};
use crate::split::peripheral::SplitPeripheral;
use crate::split::{SPLIT_MESSAGE_MAX_SIZE, SplitMessage};
use crate::state::update_status;

const SPLIT_COMPANY_ID: u16 = 0xe118;

/// Gatt service used in split peripheral to send split message to central
#[gatt_service(uuid = "4dd5fbaa-18e5-4b07-bf0a-353698659946")]
pub(crate) struct SplitBleService {
    #[characteristic(uuid = "0e6313e3-bd0b-45c2-8d2e-37a2e8128bc3", read, notify, indicate)]
    pub(crate) message_to_central: [u8; SPLIT_MESSAGE_MAX_SIZE],

    #[characteristic(uuid = "4b3514fb-cae4-4d38-a097-3a2a3d1c3b9c", write_without_response, read, notify)]
    pub(crate) message_to_peripheral: [u8; SPLIT_MESSAGE_MAX_SIZE],
}

/// Gatt server in split peripheral
#[gatt_server]
pub(crate) struct BleSplitPeripheralServer {
    pub(crate) service: SplitBleService,
}

/// BLE driver for split peripheral
pub(crate) struct BleSplitPeripheralDriver<'stack, 'server, 'c, P: PacketPool> {
    message_to_peripheral: Characteristic<[u8; SPLIT_MESSAGE_MAX_SIZE]>,
    message_to_central: Characteristic<[u8; SPLIT_MESSAGE_MAX_SIZE]>,
    conn: &'c GattConnection<'stack, 'server, P>,
}

impl<'stack, 'server, 'c, P: PacketPool> BleSplitPeripheralDriver<'stack, 'server, 'c, P> {
    pub(crate) fn new(server: &'server BleSplitPeripheralServer, conn: &'c GattConnection<'stack, 'server, P>) -> Self {
        Self {
            message_to_central: server.service.message_to_central,
            message_to_peripheral: server.service.message_to_peripheral,
            conn,
        }
    }
}

impl<'stack, 'server, 'c, P: PacketPool> SplitReader for BleSplitPeripheralDriver<'stack, 'server, 'c, P> {
    async fn read(&mut self) -> Result<SplitMessage, SplitDriverError> {
        let message = loop {
            match self.conn.next().await {
                GattConnectionEvent::Disconnected { reason } => {
                    error!("Disconnected from central: {:?}", reason);
                    update_status(|c| *c = ConnectionStatus::new());
                    return Err(SplitDriverError::Disconnected);
                }
                GattConnectionEvent::Gatt { event: gatt_event } => {
                    match &gatt_event {
                        GattEvent::Read(event) => {
                            info!("Gatt read event: {:?}", event.handle());
                        }
                        GattEvent::Write(event) => {
                            // Write to peripheral
                            if event.handle() == self.message_to_peripheral.handle {
                                let parsed = event.with_data(|_, data| {
                                    trace!("Got message from central: {:?}", data);
                                    postcard::from_bytes::<SplitMessage>(data)
                                });
                                match parsed {
                                    Ok(message) => {
                                        trace!("Message from central: {:?}", message);
                                        break message;
                                    }
                                    Err(e) => error!("Postcard deserialize split message error: {}", e),
                                }
                            } else {
                                info!("Gatt write other event: {:?}", event.handle());
                            }
                        }
                        _ => debug!("Other gatt event"),
                    };
                    match gatt_event.accept() {
                        Ok(r) => r.send().await,
                        Err(e) => warn!("[gatt] error sending response: {:?}", e),
                    }
                }
                GattConnectionEvent::ConnectionParamsUpdated {
                    conn_interval,
                    peripheral_latency,
                    supervision_timeout,
                } => {
                    info!(
                        "Connection parameters updated: {:?}ms, {:?}, {:?}ms",
                        conn_interval.as_millis(),
                        peripheral_latency,
                        supervision_timeout.as_millis()
                    );
                }
                GattConnectionEvent::PhyUpdated { tx_phy, rx_phy } => {
                    info!("PHY updated: {:?}, {:?}", tx_phy, rx_phy);
                }
                _ => (),
            }
        };
        Ok(message)
    }
}

impl<'stack, 'server, 'c, P: PacketPool> SplitWriter for BleSplitPeripheralDriver<'stack, 'server, 'c, P> {
    async fn write(&mut self, message: &SplitMessage) -> Result<usize, SplitDriverError> {
        let mut buf = [0_u8; SPLIT_MESSAGE_MAX_SIZE];
        postcard::to_slice(message, &mut buf).map_err(|e| {
            error!("Postcard serialize split message error: {}", e);
            SplitDriverError::SerializeError
        })?;
        info!("Writing split message to central: {:?}", message);
        self.message_to_central
            .notify(self.conn, &buf, true)
            .await
            .map_err(|e| {
                error!("BLE notify error: {:?}", e);
                SplitDriverError::BleError(1)
            })?;
        Ok(buf.len())
    }
}

/// Initialize and run the nRF peripheral keyboard service via BLE.
///
/// # Arguments
///
/// * `id` - The id of the peripheral
/// * `central_addr` - The address of the central
/// * `stack` - The stack to use
pub async fn initialize_nrf_ble_split_peripheral_and_run<'b, 's: 'b, C: Controller + ControllerCmdAsync<LeSetPhy>>(
    id: usize,
    stack: &'b Stack<'s, C, DefaultPacketPool>,
) {
    publish_event(CentralConnectedEvent { connected: false });

    let mut peripheral = stack.peripheral();
    let runner = stack.runner();

    // Read the previously validated split central address from storage.
    let mut central_saved = false;
    let mut central_addr = crate::storage::read_peer_address(0)
        .await
        .filter(|a| a.is_valid)
        .map(|a| {
            central_saved = true;
            a.address
        });

    let peri_task = async {
        let server = BleSplitPeripheralServer::new_default("rmk").unwrap();
        loop {
            update_status(|c| *c = ConnectionStatus::new());
            publish_event(CentralConnectedEvent { connected: false });
            match split_peripheral_advertise(id, central_addr, &mut peripheral, &server).await {
                Ok((conn, allow_rebind)) => {
                    info!("Connected to the split central");
                    let new_addr = conn.raw().peer_address().addr.into_inner();
                    if !split_central_address_allowed(central_addr, new_addr, allow_rebind) {
                        warn!("Rejecting non-paired split central address");
                        drop(conn);
                        Timer::after_millis(500).await;
                        continue;
                    }

                    let mut split_driver = BleSplitPeripheralDriver::new(&server, &conn);
                    if !validate_split_central(&mut split_driver).await {
                        warn!("Rejecting split central after product validation");
                        drop(conn);
                        Timer::after_millis(500).await;
                        continue;
                    }

                    publish_event(CentralConnectedEvent { connected: true });
                    if !central_saved || central_addr != Some(new_addr) {
                        info!("Saving validated split central address to storage");
                        if crate::storage::write_peer_address(PeerAddress {
                            peer_id: 0,
                            is_valid: true,
                            address: new_addr,
                        })
                        .await
                        {
                            central_saved = true;
                            central_addr = Some(new_addr);
                        }
                    }
                    let mut peripheral = SplitPeripheral::new(split_driver);
                    peripheral.run().await;
                    info!("Disconnected from the split central");
                }
                Err(BleHostError::BleHost(Error::Timeout)) => {
                    // Timeout, wait new keys to continue
                    error!("Connect to split central timeout");
                    let mut sub = KeyboardEvent::subscriber();
                    sub.clear();
                    let _ = sub.next_message_pure().await;
                    continue;
                }
                Err(e) => {
                    #[cfg(feature = "defmt")]
                    let e = defmt::Debug2Format(&e);
                    error!("Split advertise error: {:?}", e);
                    Timer::after_millis(500).await;
                    continue;
                }
            };
        }
    };

    join(ble_task(runner), peri_task).await;
}

async fn validate_split_central<T: SplitReader + SplitWriter>(driver: &mut T) -> bool {
    match with_timeout(Duration::from_millis(1500), driver.read()).await {
        Ok(Ok(SplitMessage::ProductId(product_id))) if product_id == crate::SPLIT_PRODUCT_ID => driver
            .write(&SplitMessage::ProductId(crate::SPLIT_PRODUCT_ID))
            .await
            .is_ok(),
        Ok(Ok(SplitMessage::ProductId(product_id))) => {
            warn!(
                "Split central product id mismatch: got {}, expected {}",
                product_id,
                crate::SPLIT_PRODUCT_ID
            );
            false
        }
        Ok(Ok(message)) => {
            warn!("Unexpected pre-handshake split message: {:?}", message);
            false
        }
        Ok(Err(e)) => {
            warn!("Split central product check read failed: {:?}", e);
            false
        }
        Err(_) => {
            warn!("Split central product check timeout");
            false
        }
    }
}

fn split_central_address_allowed(
    saved_central_addr: Option<[u8; 6]>,
    new_central_addr: [u8; 6],
    allow_rebind: bool,
) -> bool {
    saved_central_addr.is_none() || saved_central_addr == Some(new_central_addr) || allow_rebind
}

/// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
async fn split_peripheral_advertise<'a, 'b, C: Controller>(
    id: usize,
    central_addr: Option<[u8; 6]>,
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: &'b BleSplitPeripheralServer<'_>,
) -> Result<(GattConnection<'a, 'b, DefaultPacketPool>, bool), BleHostError<C::Error>> {
    let mut advertiser_data = [0; 31];

    if central_addr.is_some() {
        let advertisement = get_peri_advertiser::<C>(id, central_addr, &mut advertiser_data)?;
        let advertiser = peripheral
            .advertise(&AdvertisementParameters::default(), advertisement)
            .await?;
        match with_timeout(Duration::from_secs(10), advertiser.accept()).await {
            Ok(conn_res) => {
                let conn = conn_res?.with_attribute_server(server)?;
                info!("[adv] directed split connection established");
                return Ok((conn, false));
            }
            Err(_) => {
                warn!("[adv] directed split reconnect timeout, falling back to discoverable advertising");
            }
        }
    }

    let advertisement = get_peri_advertiser::<C>(id, None, &mut advertiser_data)?;
    let advertiser = peripheral
        .advertise(&AdvertisementParameters::default(), advertisement)
        .await?;
    match with_timeout(Duration::from_secs(10), advertiser.accept()).await {
        Ok(conn_res) => {
            let conn = conn_res?.with_attribute_server(server)?;
            info!("[adv] discoverable split connection established");
            Ok((conn, true))
        }
        Err(_) => {
            warn!("[adv] retry discoverable split advertising");
            let advertisement = get_peri_advertiser::<C>(id, None, &mut advertiser_data)?;
            let advertiser = peripheral
                .advertise(&AdvertisementParameters::default(), advertisement)
                .await?;
            match with_timeout(Duration::from_secs(300), advertiser.accept()).await {
                Ok(re) => Ok((re?.with_attribute_server(server)?, true)),
                Err(_e) => Err(BleHostError::BleHost(Error::Timeout)),
            }
        }
    }
}

fn get_peri_advertiser<'a, C: Controller>(
    id: usize,
    central_addr: Option<[u8; 6]>,
    advertiser_data: &'a mut [u8; 31],
) -> Result<Advertisement<'a>, BleHostError<C::Error>> {
    let advertisement = match central_addr {
        Some(addr) => Advertisement::ConnectableNonscannableDirected {
            peer: Address::random(addr),
        },
        None => {
            info!("No split central address provided, advertising as undirected");
            AdStructure::encode_slice(
                &[
                    AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
                    AdStructure::CompleteServiceUuids128(&[[
                        70u8, 153u8, 101u8, 152u8, 54u8, 53u8, 10u8, 191u8, 7u8, 75u8, 229u8, 24u8, 170u8, 251u8,
                        213u8, 77u8,
                    ]]),
                    AdStructure::ManufacturerSpecificData {
                        company_identifier: SPLIT_COMPANY_ID,
                        payload: &[
                            (crate::SPLIT_PRODUCT_ID & 0xff) as u8,
                            (crate::SPLIT_PRODUCT_ID >> 8) as u8,
                            id as u8,
                        ],
                    },
                ],
                &mut advertiser_data[..],
            )?;
            trace!("Split advertising data: {:?}", advertiser_data);
            Advertisement::ConnectableScannableUndirected {
                adv_data: &advertiser_data[..],
                scan_data: &[],
            }
        }
    };
    Ok(advertisement)
}

/// This is a background task that is required to run forever alongside any other BLE tasks.
async fn ble_task<C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool>(mut runner: Runner<'_, C, P>) {
    loop {
        if let Err(e) = runner.run().await {
            panic!("[ble_task] error: {:?}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::split_central_address_allowed;

    const OLD_QUBE: [u8; 6] = [1, 2, 3, 4, 5, 6];
    const NEW_QUBE: [u8; 6] = [7, 8, 9, 10, 11, 12];

    #[test]
    fn saved_qube_is_accepted_during_directed_reconnect() {
        assert!(split_central_address_allowed(Some(OLD_QUBE), OLD_QUBE, false));
    }

    #[test]
    fn different_qube_is_rejected_during_directed_reconnect() {
        assert!(!split_central_address_allowed(Some(OLD_QUBE), NEW_QUBE, false));
    }

    #[test]
    fn validated_discoverable_connection_can_rebind_qube() {
        assert!(split_central_address_allowed(Some(OLD_QUBE), NEW_QUBE, true));
    }
}
