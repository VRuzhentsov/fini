//! A plain BLE GATT link, for pre-pairing frames only: there is no pinned
//! key to dial yet, so each frame carries the sender's key itself.

use super::*;

pub struct BleDataLink {
    channel: DatagramChannel,
    peer_addr: String,
    /// A datagram already read off the channel (to route it, see
    /// `route_inbound`), returned by the first `recv`.
    pending: Option<Vec<u8>>,
}

impl BleDataLink {
    pub(super) fn new(channel: DatagramChannel) -> Self {
        let peer_addr = channel.peer().0.clone();
        Self {
            channel,
            peer_addr,
            pending: None,
        }
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub(super) fn after(channel: DatagramChannel, first: Vec<u8>) -> Self {
        Self {
            pending: Some(first),
            ..Self::new(channel)
        }
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
        // included, and `setup_hello_round` allows `FIND_PEER_CANDIDATE_TIMEOUT`
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

        let mut attempt = 0;
        loop {
            match self.channel.send(payload.clone()).await {
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
        match self.channel.recv().await? {
            Ok(bytes) => Some(Ok(bytes)),
            Err(err) => Some(Err(err.to_string())),
        }
    }
}

/// Central role: dial a peer's Bluetooth address over a plain link, for
/// Fini's own pre-pairing frames (discovery, pair request/accept/complete).
/// Sessions use `dial_session`.
pub async fn dial(address: &str) -> Result<Box<dyn DataLink>, String> {
    let backend = backend().await?;
    let peer = PeerAddress(address.to_string());
    let channel = datagram::connect(backend, &peer, &datagram_config())
        .await
        .map_err(|err| format!("ble connect to {address} failed: {err}"))?;
    Ok(Box::new(BleDataLink::new(channel)))
}
