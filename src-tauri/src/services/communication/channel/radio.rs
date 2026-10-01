//! How the Bluetooth channel actually reaches a peer.
//!
//! `BluetoothChannelService` is the policy -- when to exchange, what to
//! report. A `Radio` is the mechanism underneath it, and the service takes
//! one at construction rather than reaching for a module: that is the seam
//! that lets the same service run against real hardware and against no
//! hardware at all.
//!
//! | | reaches a peer by | used where |
//! |---|---|---|
//! | `GattRadio` | BLE advertising, scanning and GATT, via `ble-gatt` | real devices |

use async_trait::async_trait;

use crate::services::communication::pairing::DeviceConnectionState;

#[async_trait]
pub trait Radio: Send + Sync {
    /// Whether this platform has Bluetooth at all.
    #[cfg(any(feature = "ui-plane", test))]
    fn available(&self) -> bool;

    /// Ask the hardware directly, right now, whether it works.
    #[cfg(any(feature = "ui-plane", test))]
    async fn probe(&self) -> bool {
        self.available()
    }

    fn start_discovery(&self, state: &DeviceConnectionState) {
        let _ = state;
    }

    /// Start the accept loop that lets a peer connect to this device.
    #[cfg(any(feature = "ui-plane", test))]
    fn serve(&self, state: &DeviceConnectionState);

    /// See `ChannelService::keep_serving`.
    fn keep_serving(&self, state: &DeviceConnectionState) {
        let _ = state;
    }

    /// See `ChannelService::request_exchange`.
    fn request_exchange(&self, state: &DeviceConnectionState, peer_device_id: &str);

    /// See `ChannelService::forget_failures`.
    #[cfg(any(feature = "ui-plane", test))]
    fn forget_failures(&self, _peer_device_id: &str) {}

    /// Whether the peer advertised within the channel timeout.
    fn is_reachable(&self, peer_device_id: &str) -> bool;

    /// Run (or stop) the status search, for presence (ADR-0008 D12).
    #[cfg(any(feature = "ui-plane", test))]
    fn watch_presence(&self, _state: &DeviceConnectionState, _active: bool) {}

    /// The passive counterpart to `probe`: `true` until a real use of the
    /// radio failed, so an untried radio is never accused of being off.
    #[cfg(any(feature = "ui-plane", test))]
    fn adapter_available(&self) -> bool {
        true
    }
}

pub struct GattRadio;

#[async_trait]
impl Radio for GattRadio {
    #[cfg(any(feature = "ui-plane", test))]
    fn available(&self) -> bool {
        cfg!(any(target_os = "linux", target_os = "android"))
    }

    #[cfg(any(feature = "ui-plane", test))]
    async fn probe(&self) -> bool {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            super::ble::probe_adapter_available().await
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            // No adapter on this platform at all, so there is no radio to be
            // off. Answering `false` would blame the person's hardware for a
            // platform decision.
            true
        }
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn serve(&self, state: &DeviceConnectionState) {
        // Linux only, deliberately. Android starts the same loop from
        // `keep_serving`, once its Activity context exists -- starting it
        // here as well gave Android two accept loops racing each other.
        #[cfg(target_os = "linux")]
        tauri::async_runtime::spawn(super::ble::run_server(
            state.clone(),
            state.db_path.clone(),
        ));
        #[cfg(not(target_os = "linux"))]
        let _ = state;
    }

    #[cfg(not(any(feature = "ui-plane", test)))]
    #[cfg(any(feature = "ui-plane", test))]
    fn serve(&self, _state: &DeviceConnectionState) {}

    fn keep_serving(&self, state: &DeviceConnectionState) {
        // Android starts its peripheral lazily, from the first real tick:
        // the Activity context it needs does not exist when the app boots
        // (see `ble::start_peripheral_once`). Gated on the Nearby Devices
        // permission, because this runs from a background tick rather than
        // a user action -- without it `startAdvertising` throws
        // SecurityException and the loop retries forever.
        #[cfg(target_os = "android")]
        if crate::services::android_context::call_static_context_to_bool(
            "com.fini.app.BluetoothPairing",
            "hasPermissions",
        ) {
            super::ble::start_peripheral_once(state.clone(), state.db_path.clone());
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        super::ble::refresh_advertising(&state.db_path, state);
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let _ = state;
    }

    fn request_exchange(&self, state: &DeviceConnectionState, peer_device_id: &str) {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        super::ble::start_exchange(state, peer_device_id);
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let _ = (state, peer_device_id);
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn forget_failures(&self, peer_device_id: &str) {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        super::ble::forget_delivery_misses(peer_device_id);
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let _ = peer_device_id;
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn watch_presence(&self, state: &DeviceConnectionState, active: bool) {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        super::ble::set_status_search(state, active);
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let _ = (state, active);
    }

    fn is_reachable(&self, peer_device_id: &str) -> bool {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            super::ble::peer_seen_advertising_recently(peer_device_id)
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let _ = peer_device_id;
            false
        }
    }

    #[cfg(any(feature = "ui-plane", test))]
    fn adapter_available(&self) -> bool {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            !super::ble::is_bluetooth_adapter_unavailable()
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            true
        }
    }
}

pub fn for_this_device() -> Box<dyn Radio> {
    Box::new(GattRadio)
}
