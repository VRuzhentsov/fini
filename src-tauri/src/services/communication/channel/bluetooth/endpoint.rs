//! The Bluetooth channel's iroh endpoint (ADR-0009 D2, D3): dialling a
//! session or a pairing step by key, and handing what peers open to the gate.

use super::*;

/// How long a pairing dial keeps retrying a refusal caused by the candidate
/// scan's cancelled probe still tearing down its connection.
const PAIRING_CONNECT_RETRY_WINDOW: Duration = Duration::from_secs(5);

/// How long a pairing dial may take once it has the radio: its retries, one
/// connect that runs to `SESSION_CONNECT_TIMEOUT`, and opening the stream.
/// Counted from after any candidate probe it waited out, so a probe that
/// held the radio does not eat the dial's own time.
const PAIRING_CONNECT_BUDGET: Duration =
    Duration::from_secs(PAIRING_CONNECT_RETRY_WINDOW.as_secs() + SESSION_CONNECT_TIMEOUT.as_secs() + 5);

/// How long a new inbound channel may take to send its first datagram,
/// which decides what it carries (`route_inbound`).
#[cfg(any(feature = "ui-plane", test))]
const FIRST_DATAGRAM_TIMEOUT: Duration = Duration::from_secs(15);

/// How long an iroh connection over Bluetooth may take to open. A GATT
/// connect alone was measured at up to ~28 s on hardware.
const SESSION_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a BLE link stays after Fini closed the iroh connection on it, so
/// that the close and its code reach the peer first (`close_links_to`).
#[cfg(any(feature = "ui-plane", test))]
pub(crate) const TIME_BEFORE_LINK_CLOSE: Duration = Duration::from_secs(1);

/// How long a pairing's BLE link waits for the first session to use it
/// (`keep_pairing_link`).
#[cfg(any(feature = "ui-plane", test))]
const PAIRING_LINK_KEEP_TIME: Duration = Duration::from_secs(15);

impl BluetoothChannel {
    /// Opens an iroh connection to `peer_key` for a pairing step --
    /// `address` is only where to reach it first (ADR-0009 D2/D7: Bluetooth
    /// carries iroh, and iroh dials by key) -- and hands back the link plus
    /// the registrations that keep it clear of the scan: no scan runs until
    /// the caller drops the returned guards.
    ///
    /// Waits out a candidate probe already dialling, then dials within
    /// `PAIRING_CONNECT_BUDGET`.
    ///
    /// The peer's key is not pinned yet, so TLS proves only that whoever
    /// answers holds `peer_key`; pairing pins it (D8).
    pub async fn connect_for_pairing(
        &self, state: &DeviceConnectionState, peer_key: &str, address: &str,
    ) -> Result<(Box<dyn DataLink>, impl Sized), String> {
        let peer_key: iroh::EndpointId = peer_key
            .parse()
            .map_err(|err| format!("invalid key for the pairing peer: {err}"))?;
        let scanner = &self.scanner;
        let step = scanner.begin_pairing_step();
        // A candidate probe already dialling is let finish, not cancelled;
        // wait for it so this dial never overlaps one to the same peer.
        drop(scanner.candidate_probe().await);
        tokio::time::timeout(PAIRING_CONNECT_BUDGET, async {
            let connect_guard = scanner.acquire_connect().await;
            let (endpoint, transport) = self.iroh_endpoint(state).await?;
            let peer = PeerAddress(address.to_string());
            transport.set_peer_address(peer_key, peer.clone());
            let addr = iroh::EndpointAddr::from_parts(
                peer_key,
                [iroh::TransportAddr::Custom(ble_gatt_iroh::custom_addr(&peer))],
            );
            let started = tokio::time::Instant::now();
            loop {
                let attempt =
                    tokio::time::timeout(SESSION_CONNECT_TIMEOUT, endpoint.connect(addr.clone(), PEER_ALPN)).await;
                match attempt {
                    Ok(Ok(connection)) => {
                        let link =
                            IrohDataLink::open(ChannelKind::Bluetooth, connection, Some(address.to_string())).await?;
                        return Ok((Box::new(link) as Box<dyn DataLink>, connect_guard));
                    }
                    // The probe's connection may still be closing.
                    Ok(Err(_)) if started.elapsed() < PAIRING_CONNECT_RETRY_WINDOW => {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    Ok(Err(err)) => return Err(format!("bluetooth connect to {address} failed: {err}")),
                    Err(_) => return Err(format!("bluetooth connect to {address} timed out")),
                }
            }
        })
        .await
        .map_err(|_| format!("bluetooth pairing connection to {address} timed out"))?
        .map(|(link, connect_guard)| (link, (step, connect_guard)))
    }

    /// The channel's iroh endpoint over Bluetooth (ADR-0009 D2, D3): the
    /// `ble-gatt-iroh` transport only, no IP, relays off. Built on first
    /// use, which on Android has to come after the Activity is up
    /// (`BluetoothAdapter::adapter`). Its accept loop hands every connection
    /// to the gate.
    pub(crate) async fn iroh_endpoint(
        &self, state: &DeviceConnectionState,
    ) -> Result<(iroh::Endpoint, ble_gatt_iroh::BleGattTransport), String> {
        self.endpoint
            .get_or_try_init(|| async {
                let adapter = BluetoothAdapter::this_device().adapter().await?;
                let transport = ble_gatt_iroh::BleGattTransport::builder()
                    .dialer(ble_gatt_iroh::dial_with(adapter, datagram_config()))
                    .build();
                let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                    .secret_key(state.secret_key.clone())
                    .relay_mode(iroh::RelayMode::Disabled)
                    .alpns(vec![PEER_ALPN.to_vec(), SYNC_ALPN.to_vec()])
                    .transport_config(crate::services::communication::channel::iroh_link::transport_config())
                    .clear_ip_transports()
                    .add_custom_transport(Arc::new(transport.clone()))
                    .address_lookup(transport.address_lookup())
                    .bind()
                    .await
                    .map_err(|err| format!("binding the bluetooth endpoint failed: {err}"))?;
                #[cfg(any(feature = "ui-plane", test))]
                tokio::spawn(Self::serve_iroh_endpoint(state.clone(), endpoint.clone()));
                Ok::<_, String>((endpoint, transport))
            })
            .await
            .cloned()
    }

    /// Every iroh connection a peer opens over Bluetooth goes to the gate,
    /// through iroh's `Router` (`pairing::Gate`).
    #[cfg(any(feature = "ui-plane", test))]
    async fn serve_iroh_endpoint(state: DeviceConnectionState, endpoint: iroh::Endpoint) {
        crate::services::communication::pairing::Gate::serve(state, ChannelKind::Bluetooth, endpoint).await;
    }

    /// Opens an authenticated link to paired `peer_id` at `address`: an iroh
    /// connection over Bluetooth to the key pinned for the pair, which TLS
    /// proves (ADR-0009 D8). A device at that address holding another key
    /// fails the handshake, so this also confirms the candidate is the peer.
    /// `alpn`: `SYNC_ALPN` for a session, `PEER_ALPN` for `AddChannel`.
    pub async fn connect_session(
        &self, state: &DeviceConnectionState, peer_id: &str, address: &str, alpn: &[u8],
    ) -> Result<Box<dyn DataLink>, String> {
        let pinned = tokio::task::block_in_place(|| {
            crate::services::communication::pairing::pinned_key(&mut open_db_at_path(&state.db_path), peer_id)
        });
        let peer_key: iroh::EndpointId = pinned
            .ok_or_else(|| format!("{peer_id} has no key pinned; pair the devices again"))?
            .parse()
            .map_err(|err| format!("invalid key pinned for {peer_id}: {err}"))?;
        let (endpoint, transport) = self.iroh_endpoint(state).await?;
        let peer = PeerAddress(address.to_string());
        transport.set_peer_address(peer_key, peer.clone());
        let addr = iroh::EndpointAddr::from_parts(
            peer_key,
            [iroh::TransportAddr::Custom(ble_gatt_iroh::custom_addr(&peer))],
        );
        let connection = tokio::time::timeout(SESSION_CONNECT_TIMEOUT, endpoint.connect(addr, alpn))
            .await
            .map_err(|_| format!("bluetooth connect to {address} timed out"))?
            .map_err(|err| format!("bluetooth connect to {address} failed: {err}"))?;
        let link = IrohDataLink::open(ChannelKind::Bluetooth, connection, Some(address.to_string())).await?;
        Ok(Box::new(link))
    }

    /// After `after`, closes the BLE links to `peer_key`: at its current
    /// address and at the addresses it had before. A BLE link outlives the
    /// iroh connection on it until it is idle (about 25 s), and while a link
    /// to a phone's old address is up, BlueZ aborts every connect to its new
    /// one (`le-connection-abort-by-local`). Android changes the address when
    /// it leaves Add Device, so the first session after a pairing failed so.
    #[cfg(any(feature = "ui-plane", test))]
    pub(crate) async fn close_links_to(&self, peer_key: iroh::EndpointId, after: Duration) {
        let Some((_, transport)) = self.endpoint.get().cloned() else {
            return;
        };
        tokio::time::sleep(after).await;
        log::info!("[transport][ble] closing the links to {peer_key}");
        transport.close_links(&peer_key);
    }

    /// A Bluetooth session with `peer_id` ended: close its BLE links, unless
    /// a newer session with the peer is held by then, or a Bluetooth setup
    /// with it is open (its connect may be on the same link: closing it then
    /// stopped the setup). A session that the peer's side dropped left the
    /// link up here, and iroh kept writing into it while the peer had moved
    /// to a new address.
    #[cfg(any(feature = "ui-plane", test))]
    pub(crate) fn close_links_after_session(&self, state: &DeviceConnectionState, peer_id: &str) {
        let peer_key = tokio::task::block_in_place(|| {
            crate::services::communication::pairing::pinned_key(&mut open_db_at_path(&state.db_path), peer_id)
        })
        .and_then(|key| key.parse::<iroh::EndpointId>().ok());
        let Some(peer_key) = peer_key else {
            return;
        };
        let state = state.clone();
        let peer_id = peer_id.to_string();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(TIME_BEFORE_LINK_CLOSE).await;
            if state.has_session_on(&peer_id, ChannelKind::Bluetooth)
                || state.channel_setup(&peer_id, ChannelKind::Bluetooth).is_some()
            {
                return;
            }
            state.bluetooth.close_links_to(peer_key, Duration::ZERO).await;
        });
    }

    /// A pairing with `peer_id` finished on the BLE link to `address`: keep the
    /// link for the first session (`remove_pairing_link`). Not used in
    /// `PAIRING_LINK_KEEP_TIME`, it is closed: a link left up blocks the connect
    /// to the peer's new address.
    #[cfg(any(feature = "ui-plane", test))]
    pub(crate) fn keep_pairing_link(
        &self, state: &DeviceConnectionState, peer_id: &str, address: &str, peer_key: iroh::EndpointId,
    ) {
        if let Ok(mut links) = self.pairing_links.lock() {
            links.insert(peer_id.to_string(), address.to_string());
        }
        // A new pair: rejected sessions and misses from before it (the other
        // device rejected them while it was not paired yet) must not stop its
        // first session.
        self.delivery.forget_delivery_misses(peer_id);
        log::info!("[transport][ble] keeping the pairing link to {address} for the first session");
        state.space_sync.wake();
        let state = state.clone();
        let peer_id = peer_id.to_string();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(PAIRING_LINK_KEEP_TIME).await;
            let unused = state.bluetooth.remove_pairing_link(&peer_id).is_some();
            if unused && !state.has_session_on(&peer_id, ChannelKind::Bluetooth) {
                state.bluetooth.close_links_to(peer_key, Duration::ZERO).await;
            }
        });
    }

    /// Whether a pairing link is kept for `peer_id`'s first session.
    pub(crate) fn has_pairing_link(&self, peer_id: &str) -> bool {
        self.pairing_links.lock().is_ok_and(|links| links.contains_key(peer_id))
    }

    /// The address of the pairing link kept for `peer_id`'s first session,
    /// once: a later session searches as usual.
    pub(crate) fn remove_pairing_link(&self, peer_id: &str) -> Option<String> {
        self.pairing_links.lock().ok()?.remove(peer_id)
    }

    /// The gate rejected a Bluetooth session from `address` (not paired, or
    /// the channel is off or missing). Its BLE link stayed up until idle, and
    /// iroh sent this device's own next connection to the peer into it
    /// instead of connecting: the connect timed out. No session to the peer is
    /// kept on this channel then, so the link is closed after the rejection
    /// gets to the peer -- unless a setup with `device_id` is open: its
    /// connect can use the same link.
    #[cfg(any(feature = "ui-plane", test))]
    pub(crate) fn close_rejected_link(&self, state: &DeviceConnectionState, address: String, device_id: Option<&str>) {
        if address.is_empty() {
            return;
        }
        let state = state.clone();
        let device_id = device_id.map(str::to_string);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(TIME_BEFORE_LINK_CLOSE).await;
            if device_id.is_some_and(|id| state.has_channel_setup(&id, ChannelKind::Bluetooth)) {
                return;
            }
            state.bluetooth.close_link_at(address, Duration::ZERO).await;
        });
    }

    /// Like `close_links_to`, for a link the peer opened from `address`,
    /// which this device never connected to.
    #[cfg(any(feature = "ui-plane", test))]
    pub(crate) async fn close_link_at(&self, address: String, after: Duration) {
        let Some((_, transport)) = self.endpoint.get().cloned() else {
            return;
        };
        tokio::time::sleep(after).await;
        log::info!("[transport][ble] closing the link to {address}");
        transport.close_link(&PeerAddress(address));
    }

    /// Routes a channel a central opened by its first datagram: a QUIC
    /// Initial goes to the iroh transport (an authenticated session),
    /// anything else is one of Fini's own pre-pairing frames on a plain
    /// link.
    #[cfg(any(feature = "ui-plane", test))]
    pub(super) async fn route_inbound(
        &self, state: &DeviceConnectionState, db_path: PathBuf, mut channel: DatagramChannel,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let first = match tokio::time::timeout(FIRST_DATAGRAM_TIMEOUT, channel.recv()).await {
            Ok(Some(Ok(first))) => first,
            _ => return None,
        };
        if ble_gatt_iroh::is_quic_initial(&first) {
            match self.iroh_endpoint(state).await {
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
}
