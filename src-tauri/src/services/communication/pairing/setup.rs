//! Setting a channel up for an already-paired device (ADR-0008 D1-D4).
//!
//! Both people open the setup dialog, and each device runs a setup search:
//! it says hello to the peer on that channel and answers the peer's hello
//! (the gate does that, while this search runs and afterwards while the
//! channel exists -- ADR-0008 D2). The channel's
//! init is complete on a device once both halves happened there -- its own
//! hello was acknowledged, and it acknowledged the peer's. Only then can
//! the person press OK, which switches the channel on for this device
//! alone.
//!
//! Methods of `DeviceConnectionState`, because the setup records are held
//! by the state. How a hello finds the peer is the channel's own
//! (`ChannelService::setup_round`).

use std::time::Duration;

use diesel::SqliteConnection;

use super::{channels, ChannelKind, ChannelSetup, DeviceConnectionState};

impl DeviceConnectionState {
    /// Start (or keep) the setup search for this peer's channel. The search
    /// runs until `finish_channel_setup` ends it.
    pub fn start_channel_setup(&self, peer_device_id: &str, kind: ChannelKind) {
        let already_running = self.channel_setup(peer_device_id, kind).is_some();
        let attempt = self.begin_channel_setup(peer_device_id, kind);
        // The peer must be able to reach this device while it searches, so a
        // setup is a reason to advertise (ADR-0008 D8); the keeper applies it.
        self.space_sync.wake();
        if already_running {
            return;
        }
        let state = self.clone();
        let peer_device_id = peer_device_id.to_string();
        tauri::async_runtime::spawn(async move { state.run_channel_setup(&peer_device_id, kind, attempt).await });
    }

    /// End the setup search. If the init completed, the channel is created
    /// `On` when the person pressed OK and `Off` when they closed the dialog
    /// without it (ADR-0008 D15); otherwise nothing is written. Closing
    /// never switches off a channel that already exists -- the code
    /// fallback may have set it up `On` while the automatic search ran.
    pub fn finish_channel_setup(
        &self,
        conn: &mut SqliteConnection,
        peer_device_id: &str,
        kind: ChannelKind,
        switch_on: bool,
    ) -> Result<Option<ChannelSetup>, String> {
        // Ended only once the channel is written: if writing fails, the init
        // survives for another try instead of costing both people a new one.
        let setup = self.channel_setup(peer_device_id, kind);
        if setup.is_some_and(|setup| setup.initialized())
            && (switch_on || channels::find(conn, peer_device_id, kind).is_none())
        {
            channels::configure(conn, peer_device_id, kind, switch_on, None)?;
        }
        self.end_channel_setup(peer_device_id, kind);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if kind == ChannelKind::Bluetooth && setup.is_some_and(|setup| setup.initialized()) {
            self.bluetooth.advertiser.keep_answering_after_setup(&self.space_sync);
        }
        self.space_sync.wake();
        Ok(setup)
    }

    /// Says hello until the peer acknowledges it or this attempt ends.
    async fn run_channel_setup(&self, peer_device_id: &str, kind: ChannelKind, attempt: u64) {
        // The dialog also runs add mode for a known device, and its candidate
        // scan re-dials the very phone this search is saying hello to.
        // Crossing dials fail each other's GATT setup, so the candidate scan
        // stands aside for the whole setup -- not only until our own hello is
        // acknowledged: the peer's hello still has to reach this device
        // afterwards, and this device's scan dialling it in the meantime
        // keeps that from happening.
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let _clear_of_candidate_scan =
            (kind == ChannelKind::Bluetooth).then(|| self.bluetooth.scanner.begin_pairing_step());
        self.send_channel_request(peer_device_id, kind, attempt).await;
        if kind == ChannelKind::Bluetooth {
            while self
                .channel_setup(peer_device_id, kind)
                .is_some_and(|setup| setup.attempt == attempt)
            {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }

    async fn send_channel_request(&self, peer_device_id: &str, kind: ChannelKind, attempt: u64) {
        let channel = self.channel(kind);
        loop {
            match self.channel_setup(peer_device_id, kind) {
                Some(setup) if setup.attempt == attempt && !setup.sent_request_accepted => {}
                _ => return,
            }
            if channel.setup_round(self, peer_device_id).await {
                self.note_channel_setup_attempt(peer_device_id, kind, attempt, |setup| {
                    setup.sent_request_accepted = true
                });
                return;
            }
        }
    }
}
