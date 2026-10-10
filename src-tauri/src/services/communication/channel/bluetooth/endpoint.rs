//! This state's iroh endpoint over Bluetooth (ADR-0009 D2, D3): dialling a
//! session or a pairing leg by key, and handing what peers open to the gate.

use super::*;

/// How long a pairing dial keeps retrying a refusal caused by the candidate
/// scan's cancelled probe still tearing down its connection.
const PAIRING_DIAL_RETRY_WINDOW: Duration = Duration::from_secs(5);

/// Opens an iroh connection to `peer_key` for a pairing leg -- `address` is
/// only where to reach it first (ADR-0009 D2/D7: Bluetooth carries iroh, and
/// iroh dials by key) -- and hands back the link plus the registrations that
/// keep it clear of the candidate scan: a pass stops at its next step, and no
/// scan runs until the caller drops the returned guards.
///
/// The peer's key is not pinned yet, so TLS proves only that whoever answers
/// holds `peer_key`; pairing pins it (D8).
pub async fn dial_for_pairing(
    state: &DeviceConnectionState, peer_key: &str, address: &str,
) -> Result<(Box<dyn DataLink>, impl Sized), String> {
    let peer_key: iroh::EndpointId = peer_key
        .parse()
        .map_err(|err| format!("invalid key for the pairing peer: {err}"))?;
    let radio = &state.bluetooth_radio;
    let leg = radio.begin_pairing_leg();
    // A candidate probe already dialling is let finish, not cancelled; wait
    // for it so this dial never overlaps one to the same peer.
    drop(radio.candidate_probe().await);
    let dial_guard = radio.acquire_dial().await;
    let (endpoint, transport) = bluetooth_endpoint(state).await?;
    let peer = PeerAddress(address.to_string());
    transport.set_peer_address(peer_key, peer.clone());
    let addr = iroh::EndpointAddr::from_parts(
        peer_key,
        [iroh::TransportAddr::Custom(ble_gatt_iroh::custom_addr(&peer))],
    );
    let started = tokio::time::Instant::now();
    loop {
        let attempt = tokio::time::timeout(SESSION_CONNECT_TIMEOUT, endpoint.connect(addr.clone(), ALPN)).await;
        match attempt {
            Ok(Ok(connection)) => {
                let link = IrohDataLink::open(ChannelKind::Bluetooth, connection, Some(address.to_string())).await?;
                return Ok((Box::new(link) as Box<dyn DataLink>, (leg, dial_guard)));
            }
            // The probe's connection may still be closing.
            Ok(Err(_)) if started.elapsed() < PAIRING_DIAL_RETRY_WINDOW => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Ok(Err(err)) => return Err(format!("bluetooth connect to {address} failed: {err}")),
            Err(_) => return Err(format!("bluetooth connect to {address} timed out")),
        }
    }
}

/// How long a new inbound channel may take to send its first datagram,
/// which decides what it carries (`route_inbound`).
#[cfg(any(feature = "ui-plane", test))]
const FIRST_DATAGRAM_TIMEOUT: Duration = Duration::from_secs(15);

/// How long an iroh connection over Bluetooth may take to open. A GATT
/// connect alone was measured at up to ~28 s on hardware.
const SESSION_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// This state's Bluetooth iroh endpoint (ADR-0009 D2, D3): the
/// `ble-gatt-iroh` transport only, no IP, relays off. Built on first use,
/// which on Android has to come after the Activity is up (`backend`). Its
/// accept loop hands every connection to the gate.
pub(crate) async fn bluetooth_endpoint(
    state: &DeviceConnectionState,
) -> Result<(iroh::Endpoint, ble_gatt_iroh::BleGattTransport), String> {
    state
        .bluetooth_endpoint
        .get_or_try_init(|| async {
            let backend = backend().await?;
            let transport = ble_gatt_iroh::BleGattTransport::builder()
                .dialer(ble_gatt_iroh::dial_with(backend, datagram_config()))
                .build();
            let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .secret_key(state.secret_key.clone())
                .relay_mode(iroh::RelayMode::Disabled)
                .alpns(vec![ALPN.to_vec()])
                .clear_ip_transports()
                .add_custom_transport(Arc::new(transport.clone()))
                .address_lookup(transport.address_lookup())
                .bind()
                .await
                .map_err(|err| format!("binding the bluetooth endpoint failed: {err}"))?;
            #[cfg(any(feature = "ui-plane", test))]
            tokio::spawn(serve_bluetooth_endpoint(state.clone(), endpoint.clone()));
            Ok::<_, String>((endpoint, transport))
        })
        .await
        .cloned()
}

/// Every iroh connection a peer opens over Bluetooth goes to the gate.
#[cfg(any(feature = "ui-plane", test))]
async fn serve_bluetooth_endpoint(state: DeviceConnectionState, endpoint: iroh::Endpoint) {
    while let Some(incoming) = endpoint.accept().await {
        let peer_addr = match incoming.remote_addr() {
            iroh::endpoint::IncomingAddr::Custom(addr) => ble_gatt_iroh::peer_address(&addr).map(|peer| peer.0),
            _ => None,
        };
        let state = state.clone();
        tokio::spawn(async move {
            let connection = match incoming.accept() {
                Ok(accepting) => match accepting.await {
                    Ok(connection) => connection,
                    Err(err) => {
                        log::warn!("[transport][ble] handshake failed: {err}");
                        return;
                    }
                },
                Err(err) => {
                    log::warn!("[transport][ble] accept failed: {err}");
                    return;
                }
            };
            match IrohDataLink::accept(ChannelKind::Bluetooth, connection, peer_addr).await {
                Ok(link) => {
                    let db_path = state.db_path.clone();
                    crate::services::communication::pairing::run_peer_gate(Box::new(link), state, db_path).await;
                }
                Err(err) => log::warn!("[transport][ble] {err}"),
            }
        });
    }
}

/// Opens an authenticated link to paired `peer_id` at `address`: an iroh
/// connection over Bluetooth to the key pinned for the pair, which TLS
/// proves (ADR-0009 D8). A device at that address holding another key fails
/// the handshake, so this also confirms the candidate is the peer.
pub async fn dial_session(
    state: &DeviceConnectionState, peer_id: &str, address: &str,
) -> Result<Box<dyn DataLink>, String> {
    let pinned = tokio::task::block_in_place(|| {
        crate::services::communication::pairing::pinned_key(&mut open_db_at_path(&state.db_path), peer_id)
    });
    let peer_key: iroh::EndpointId = pinned
        .ok_or_else(|| format!("{peer_id} has no key pinned; pair the devices again"))?
        .parse()
        .map_err(|err| format!("invalid key pinned for {peer_id}: {err}"))?;
    let (endpoint, transport) = bluetooth_endpoint(state).await?;
    let peer = PeerAddress(address.to_string());
    transport.set_peer_address(peer_key, peer.clone());
    let addr = iroh::EndpointAddr::from_parts(
        peer_key,
        [iroh::TransportAddr::Custom(ble_gatt_iroh::custom_addr(&peer))],
    );
    let connection = tokio::time::timeout(SESSION_CONNECT_TIMEOUT, endpoint.connect(addr, ALPN))
        .await
        .map_err(|_| format!("bluetooth connect to {address} timed out"))?
        .map_err(|err| format!("bluetooth connect to {address} failed: {err}"))?;
    let link = IrohDataLink::open(ChannelKind::Bluetooth, connection, Some(address.to_string())).await?;
    Ok(Box::new(link))
}

/// Routes a channel a central opened by its first datagram: a QUIC Initial
/// goes to the iroh transport (an authenticated session), anything else is
/// one of Fini's own pre-pairing frames on a plain link.
#[cfg(any(feature = "ui-plane", test))]
pub(super) async fn route_inbound(
    state: &DeviceConnectionState, db_path: PathBuf, mut channel: DatagramChannel,
) -> Option<tokio::task::JoinHandle<()>> {
    let first = match tokio::time::timeout(FIRST_DATAGRAM_TIMEOUT, channel.recv()).await {
        Ok(Some(Ok(first))) => first,
        _ => return None,
    };
    if ble_gatt_iroh::is_quic_initial(&first) {
        match bluetooth_endpoint(state).await {
            Ok((_, transport)) => Some(transport.attach_after(channel, first)),
            Err(err) => {
                log::warn!("[transport][ble] cannot take a session: {err}");
                None
            }
        }
    } else {
        let link: Box<dyn DataLink> = Box::new(BleDataLink::after(channel, first));
        let state = state.clone();
        Some(tokio::spawn(crate::services::communication::pairing::run_peer_gate(link, state, db_path)))
    }
}

