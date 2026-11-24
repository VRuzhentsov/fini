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
const PAIRING_CONNECT_BUDGET: Duration = Duration::from_secs(
    PAIRING_CONNECT_RETRY_WINDOW.as_secs() + SESSION_CONNECT_TIMEOUT.as_secs() + 5,
);

/// How long a new inbound channel may take to send its first datagram,
/// which decides what it carries (`route_inbound`).
const FIRST_DATAGRAM_TIMEOUT: Duration = Duration::from_secs(15);

/// How long an iroh connection over Bluetooth may take to open. A GATT
/// connect alone was measured at up to ~28 s on hardware.
const SESSION_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a pairing's BLE link waits for the first session to use it
/// (`keep_pairing_link`).
const PAIRING_LINK_KEEP_TIME: Duration = Duration::from_secs(15);

#[allow(
    clippy::multiple_inherent_impl,
    reason = "this submodule owns the iroh endpoint part of the Bluetooth channel"
)]
impl BluetoothChannelService {
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
        &self,
        peer_key: &str,
        address: &str,
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
            let (endpoint, transport) = self.iroh_endpoint().await?;
            let peer = PeerAddress(address.to_string());
            transport.set_peer_address(peer_key, peer.clone());
            let addr = iroh::EndpointAddr::from_parts(
                peer_key,
                [iroh::TransportAddr::Custom(ble_gatt_iroh::custom_addr(
                    &peer,
                ))],
            );
            let started = tokio::time::Instant::now();
            loop {
                let attempt = tokio::time::timeout(
                    SESSION_CONNECT_TIMEOUT,
                    endpoint.connect(addr.clone(), PEER_ALPN),
                )
                .await;
                match attempt {
                    Ok(Ok(connection)) => {
                        let link = IrohDataLink::open(
                            ChannelKind::Bluetooth,
                            connection,
                            Some(address.to_string()),
                        )
                        .await?;
                        return Ok((Box::new(link) as Box<dyn DataLink>, connect_guard));
                    }
                    // The probe's connection may still be closing.
                    Ok(Err(_)) if started.elapsed() < PAIRING_CONNECT_RETRY_WINDOW => {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    Ok(Err(err)) => {
                        return Err(format!("bluetooth connect to {address} failed: {err}"))
                    }
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
    /// (`BluetoothAdapter::adapter`). Its `Router` hands every connection to
    /// the handler of its ALPN.
    pub(crate) async fn iroh_endpoint(
        &self,
    ) -> Result<(iroh::Endpoint, ble_gatt_iroh::BleGattTransport), String> {
        let shared = self.peer_link().await?;
        let _initialization = self.initialization.write().await;
        if self.stopping() {
            return Err("Bluetooth is stopping".to_string());
        }
        if let Some(endpoint) = self.endpoint.get() {
            return Ok(endpoint.clone());
        }
        let transport = ble_gatt_iroh::BleGattTransport::builder()
            .dialer(ble_gatt_iroh::dial_with(shared))
            .build();
        let bind = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .secret_key(self.identity.secret_key.clone())
            .relay_mode(iroh::RelayMode::Disabled)
            .alpns(vec![PEER_ALPN.to_vec(), SYNC_ALPN.to_vec()])
            .transport_config(crate::services::channel_service::iroh_link::transport_config())
            .clear_ip_transports()
            .add_custom_transport(Arc::new(transport.clone()))
            .hooks(transport.hooks())
            .address_lookup(transport.address_lookup())
            .bind();
        let endpoint = tokio::select! {
            biased;
            () = self.wait_until_stopping() => return Err("Bluetooth is stopping".to_string()),
            endpoint = bind => endpoint,
        }
        .map_err(|err| format!("binding the Bluetooth endpoint failed: {err}"))?;
        let published = {
            let _admission = self.admission.lock().map_err(|err| err.to_string())?;
            if self.stopping() {
                false
            } else {
                self.endpoint
                    .set((endpoint.clone(), transport.clone()))
                    .map_err(|_| "Bluetooth endpoint already initialized".to_string())?;
                match self.handlers.get().cloned() {
                    Some(handlers) => {
                        tokio::spawn(crate::services::channel_service::LinkAcceptor::serve(
                            ChannelKind::Bluetooth,
                            endpoint.clone(),
                            handlers,
                            Arc::new(|| false),
                        ));
                    }
                    None => {
                        log::warn!(
                            "[transport][ble] no link handlers; the endpoint answers nobody"
                        );
                    }
                }
                true
            }
        };
        if !published {
            return match tokio::time::timeout(Duration::from_secs(2), endpoint.close()).await {
                Ok(()) => Err("Bluetooth stopped during endpoint initialization".to_string()),
                Err(err) => Err(format!(
                    "Bluetooth endpoint initialization close timed out: {err}"
                )),
            };
        }
        Ok((endpoint, transport))
    }

    /// Opens an authenticated link to paired `peer_id` at `address`: an iroh
    /// connection over Bluetooth to the key pinned for the pair, which TLS
    /// proves (ADR-0009 D8). A device at that address holding another key
    /// fails the handshake, so this also confirms the candidate is the peer.
    /// `alpn`: `SYNC_ALPN` for a session, `PEER_ALPN` for `AddChannel`.
    pub async fn connect_session(
        &self,
        peer_id: &str,
        address: &str,
        alpn: &[u8],
    ) -> Result<Box<dyn DataLink>, String> {
        let peer_key: iroh::EndpointId = self
            .pinned_key(peer_id)
            .ok_or_else(|| format!("{peer_id} has no key pinned; pair the devices again"))?
            .parse()
            .map_err(|err| format!("invalid key pinned for {peer_id}: {err}"))?;
        let (endpoint, transport) = self.iroh_endpoint().await?;
        let peer = PeerAddress(address.to_string());
        transport.set_peer_address(peer_key, peer.clone());
        let addr = iroh::EndpointAddr::from_parts(
            peer_key,
            [iroh::TransportAddr::Custom(ble_gatt_iroh::custom_addr(
                &peer,
            ))],
        );
        let connection =
            tokio::time::timeout(SESSION_CONNECT_TIMEOUT, endpoint.connect(addr, alpn))
                .await
                .map_err(|_| format!("bluetooth connect to {address} timed out"))?
                .map_err(|err| format!("bluetooth connect to {address} failed: {err}"))?;
        let link = IrohDataLink::open(
            ChannelKind::Bluetooth,
            connection,
            Some(address.to_string()),
        )
        .await?;
        Ok(Box::new(link))
    }

    /// Closes the BLE links to `peer_key`: at its current address and at the
    /// addresses it had before. A BLE link outlives the iroh connection on
    /// it until it is idle (about 25 s), and while a link to a phone's old
    /// address is up, BlueZ aborts every connect to its new one
    /// (`le-connection-abort-by-local`). Android changes the address when it
    /// leaves Add Device, so the first session after a pairing failed so.
    pub(crate) fn close_links_to(&self, peer_key: iroh::EndpointId) {
        let held = self
            .pairing_links
            .lock()
            .ok()
            .map(|mut links| {
                let peers: Vec<_> = links
                    .iter()
                    .filter(|(_, (_, key, _))| *key == peer_key)
                    .map(|(peer, _)| peer.clone())
                    .collect();
                peers
                    .into_iter()
                    .filter_map(|peer| links.remove(&peer).map(|(_, _, channel)| channel))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for channel in held {
            tauri::async_runtime::spawn(async move {
                if let Err(err) = channel.disconnect().await {
                    log::warn!("[transport][ble] pairing channel disconnect failed: {err}");
                }
                if let Err(err) = channel.release().await {
                    log::warn!("[transport][ble] pairing use release failed: {err}");
                }
            });
        }
        let Some((_, transport)) = self.endpoint.get().cloned() else {
            return;
        };
        log::info!("[transport][ble] closing the links to {peer_key}");
        transport.close_links(&peer_key);
        transport.forget_peer(&peer_key);
    }

    /// Keep a scoped use until the first session acquires the pairing channel.
    pub(crate) fn keep_pairing_link(
        &self,
        peer_id: &str,
        address: &str,
        peer_key: iroh::EndpointId,
    ) {
        if self.stopping() {
            return;
        }
        let Some(channel) = self
            .peer_link
            .get()
            .and_then(|link| link.channel(&PeerAddress(address.to_string())))
        else {
            log::warn!("[transport][ble] pairing channel is already closed");
            return;
        };
        let token = Uuid::new_v4();
        let previous = self.pairing_links.lock().ok().and_then(|mut links| {
            if self.stopping() {
                return None;
            }
            links.insert(peer_id.to_string(), (token, peer_key, channel))
        });
        if let Some((_, _, channel)) = previous {
            tauri::async_runtime::spawn(async move {
                if let Err(err) = channel.release().await {
                    log::warn!("[transport][ble] old pairing use release failed: {err}");
                }
            });
        }
        self.delivery.forget_delivery_misses(peer_id);
        self.bus.publish(Event::DeviceHeard {
            peer_device_id: peer_id.to_string(),
            kind: ChannelKind::Bluetooth,
        });
        let owner = self.me.clone();
        let peer_id = peer_id.to_string();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(PAIRING_LINK_KEEP_TIME).await;
            let Some(owner) = owner.upgrade() else { return };
            let expired = owner.pairing_links.lock().ok().and_then(|mut links| {
                if links
                    .get(&peer_id)
                    .is_some_and(|(current, _, _)| *current == token)
                {
                    links.remove(&peer_id).map(|(_, _, channel)| channel)
                } else {
                    None
                }
            });
            if let Some(channel) = expired {
                if let Err(err) = channel.release().await {
                    log::warn!("[transport][ble] pairing use release failed: {err}");
                }
            }
        });
    }

    pub(crate) fn has_pairing_link(&self, peer_id: &str) -> bool {
        self.pairing_links
            .lock()
            .is_ok_and(|links| links.contains_key(peer_id))
    }

    pub(crate) fn remove_pairing_link(&self, peer_id: &str) -> Option<PeerChannel> {
        self.pairing_links
            .lock()
            .ok()?
            .remove(peer_id)
            .map(|(_, _, channel)| channel)
    }

    /// An identify step answered with the peer's key: its plain link goes to
    /// the iroh transport, so that the pairing connection that follows runs
    /// on it. A second BLE connection cost about 5 s (pair-1: 2 s waiting
    /// for the first link to go, then connect and subscribe again).
    pub(crate) async fn keep_identify_link(&self, link: BleDataLink, endpoint_id: &str) {
        let Ok(peer_key) = endpoint_id.parse::<iroh::EndpointId>() else {
            return;
        };
        let Ok((_, transport)) = self.iroh_endpoint().await else {
            return;
        };
        let address = PeerAddress(link.peer_addr().unwrap_or_default());
        let Some(channel) = link.into_channel() else {
            return;
        };
        log::info!(
            "[transport][ble] keeping the identify link to {} for the pairing connection",
            address.0
        );
        transport.set_peer_address(peer_key, address);
        transport.attach(channel);
    }

    /// Routes a channel a central opened by its first datagram: a QUIC
    /// Initial goes to the iroh transport (an authenticated session),
    /// anything else is one of Fini's own pre-pairing frames on a plain
    /// link.
    pub(super) async fn route_inbound(
        &self,
        channel: PeerChannel,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let first = match tokio::time::timeout(FIRST_DATAGRAM_TIMEOUT, channel.recv()).await {
            Ok(Some(Ok(first))) => first,
            outcome => {
                log::info!(
                    "[transport][ble] first frame ended: {outcome:?}; reason {:?}",
                    channel.close_reason()
                );
                if let Err(err) = channel.release().await {
                    log::warn!("[transport][ble] incoming channel release failed: {err}");
                }
                return None;
            }
        };
        if ble_gatt_iroh::is_quic_initial(&first) {
            match self.iroh_endpoint().await {
                Ok((_, transport)) => Some(transport.attach_after(channel, first)),
                Err(err) => {
                    log::warn!("[transport][ble] cannot take a session: {err}");
                    if let Err(err) = channel.release().await {
                        log::warn!("[transport][ble] incoming channel release failed: {err}");
                    }
                    None
                }
            }
        } else {
            let session = self
                .iroh_endpoint()
                .await
                .ok()
                .map(|(_, transport)| transport);
            let link: Box<dyn DataLink> = Box::new(BleDataLink::after(channel, first, session));
            let handlers = self.handlers.get().cloned()?;
            Some(tokio::spawn(
                async move { handlers.peer.accept(link).await },
            ))
        }
    }
}
