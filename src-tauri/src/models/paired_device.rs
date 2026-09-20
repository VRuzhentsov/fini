use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::paired_devices;

#[derive(Debug, Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = paired_devices)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct PairedDevice {
    pub peer_device_id: String,
    pub display_name: String,
    pub paired_at: String,
    pub last_seen_at: Option<String>,
    pub pair_state: String,
    pub bluetooth_enabled: bool,
    pub bluetooth_address: Option<String>,
    pub bluetooth_last_verified_at: Option<String>,
    /// Set when the user explicitly turns Bluetooth *off* for this pair via
    /// the Device settings toggle, cleared when they explicitly turn it
    /// back on -- distinct from `bluetooth_enabled` itself, which
    /// `persist_bluetooth_address_and_maybe_enable` also flips off for
    /// unrelated reasons (an address that isn't OS-bonded, an inconclusive
    /// check). Without this separate flag, a later self-reported address
    /// update over an authenticated session would happily re-confirm the
    /// bond and turn Bluetooth back on, silently undoing an explicit
    /// disable the moment the peer reconnects (`specs/device-connect/README.md`'s
    /// disable contract).
    pub bluetooth_disabled_by_user: bool,
    /// A manually-pinned transport ("network"/"bluetooth",
    /// `pairing::channel_status::TransportKind`'s serde form), or
    /// `None` for pure automatic network-first primary selection -- every
    /// pre-existing row's default. Both transports stay connected
    /// regardless of this pin (ADR-0003 revision); it only decides which
    /// already-connected one is primary. Persisted so it also governs
    /// *future* automatic reconnects, not just the one that set it.
    pub preferred_transport: Option<String>,
    /// When `preferred_transport` was last set.
    pub preferred_transport_set_at: Option<String>,
    /// The per-pair Network switch, the counterpart to `bluetooth_enabled`.
    ///
    /// Defaults to `true`, where Bluetooth defaults to `false`: Network is
    /// the channel a pair is normally formed over and is already carrying
    /// traffic, so the switch exists to let a user stop it deliberately,
    /// not to make them opt in to what already works.
    ///
    /// Turning it off keeps the pair, the trust and the mapped spaces
    /// intact -- it only stops this device dialling the peer over the
    /// network and stops presence counting as a reason to connect.
    pub network_enabled: bool,
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = paired_devices)]
pub struct CreatePairedDeviceInput {
    pub peer_device_id: String,
    pub display_name: String,
    pub paired_at: String,
}
