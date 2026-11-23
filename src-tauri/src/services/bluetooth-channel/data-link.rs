//! A plain BLE GATT link, for pre-pairing frames only: there is no pinned
//! key to dial yet, so each frame carries the sender's key itself.

use super::*;

pub struct BleDataLink {
    /// `None` once the channel went to the iroh transport.
    channel: Option<PeerChannel>,
    peer_addr: String,
    /// A datagram already read off the channel (to route it, see
    /// `route_inbound`), returned by the first `recv`.
    pending: Option<Vec<u8>>,
    /// Takes the channel when a QUIC Initial arrives on it: the identifying
    /// side starts its pairing connection on the identify link, in place of
    /// a second BLE connection.
    session: Option<ble_gatt_iroh::BleGattTransport>,
}

impl Drop for BleDataLink {
    fn drop(&mut self) {
        if let Some(channel) = self.channel.take() {
            tauri::async_runtime::spawn(async move {
                if let Err(err) = channel.release().await {
                    log::warn!("[transport][ble] plain channel release failed: {err}");
                }
            });
        }
    }
}

impl BleDataLink {
    pub(super) fn new(channel: PeerChannel) -> Self {
        let peer_addr = channel.peer().0;
        Self {
            channel: Some(channel),
            peer_addr,
            pending: None,
            session: None,
        }
    }

    pub(super) fn after(
        channel: PeerChannel,
        first: Vec<u8>,
        session: Option<ble_gatt_iroh::BleGattTransport>,
    ) -> Self {
        let mut link = Self::new(channel);
        link.pending = Some(first);
        link.session = session;
        link
    }

    /// The channel, for the iroh transport (`keep_identify_link`).
    pub(super) fn into_channel(mut self) -> Option<PeerChannel> {
        self.channel.take()
    }
}

#[async_trait]
impl DataLink for BleDataLink {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Bluetooth
    }

    fn peer_addr(&self) -> Option<String> {
        Some(self.peer_addr.clone())
    }

    async fn send(&mut self, payload: Vec<u8>) -> Result<(), String> {
        // Retry `GattBusy` here rather than inside `ble-gatt`: the backend
        // makes exactly one attempt and classifies a rejection it believes is
        // transient as `BleError::GattBusy`, deliberately leaving the retry
        // budget to whoever knows the caller's own deadline. No single
        // backend-side budget can be right for every caller -- a chain sized
        // for a generous one silently exceeds a tight one and reads as a
        // caller-abandoned future rather than an honest failure.
        //
        // Sized to the tightest caller on this path: `scan_add_mode_candidates`
        // gets `BLUETOOTH_SCAN_DURATION_MS` (4s) for the *whole* pass, dial
        // included, and `BluetoothChannel::find_and_send_request` allows `FIND_PEER_CANDIDATE_TIMEOUT`
        // (4s) per candidate. With a dial typically eating 1-2s of that, the
        // ~1.4s worst case below still leaves the caller room to fail cleanly
        // instead of being cut off mid-retry.
        //
        // Observed on real hardware: the *first* write on a freshly connected
        // channel is the one that gets rejected (msg_id=0, fragment 0), while
        // the `subscribe` moments earlier on the same link succeeds -- so this
        // covers a genuine just-connected window, not a dead peer.
        const SEND_RETRY_DELAYS: [Duration; 3] = [
            Duration::from_millis(150),
            Duration::from_millis(300),
            Duration::from_millis(600),
        ];

        let Some(channel) = self.channel.as_mut() else {
            return Err("the link went to the iroh transport".to_string());
        };
        let mut attempt = 0;
        loop {
            match channel.send(payload.clone()).await {
                Ok(()) => return Ok(()),
                Err(ble_gatt::BleError::GattBusy(err)) if attempt < SEND_RETRY_DELAYS.len() => {
                    log::warn!(
                        "[transport][ble] send to {} rejected as busy ({err}), retrying ({}/{})",
                        self.peer_addr,
                        attempt + 1,
                        SEND_RETRY_DELAYS.len()
                    );
                    tokio::time::sleep(SEND_RETRY_DELAYS[attempt]).await;
                    attempt += 1;
                }
                Err(err) => return Err(err.to_string()),
            }
        }
    }

    async fn recv(&mut self) -> Option<Result<Vec<u8>, String>> {
        if let Some(first) = self.pending.take() {
            return Some(Ok(first));
        }
        let bytes = match self.channel.as_mut()?.recv().await? {
            Ok(bytes) => bytes,
            Err(err) => return Some(Err(err.to_string())),
        };
        // The identifying side's pairing connection: the channel goes to
        // iroh, and this link ends here.
        if ble_gatt_iroh::is_quic_initial(&bytes) {
            if let (Some(session), Some(channel)) = (self.session.as_ref(), self.channel.take()) {
                log::info!(
                    "[transport][ble] {}: the pairing connection goes on the identify link",
                    self.peer_addr
                );
                session.attach_after(channel, bytes);
                return None;
            }
        }
        Some(Ok(bytes))
    }
}

/// Central role: connect to a peer's Bluetooth address over a plain link,
/// for Fini's own pre-pairing frames (identify, pair request/accept/
/// complete). Sessions use `BluetoothChannel::connect_session`.
impl BleDataLink {
    pub(super) async fn connect(shared: Arc<PeerLink>, address: &str) -> Result<Self, String> {
        let peer = PeerAddress(address.to_string());
        let channel = shared
            .request(&peer)
            .await
            .map_err(|err| err.to_string())?
            .accept()
            .await
            .map_err(|err| format!("ble connect to {address} failed: {err}"))?;
        Ok(BleDataLink::new(channel))
    }
}
