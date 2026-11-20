//! The Tauri commands of the channel services.

use std::sync::Arc;

use tauri::State;

use crate::services::bluetooth_channel_service::BluetoothChannelService;
use crate::services::channel_service::ChannelService;

/// Whether this device's Bluetooth radio can be used now, asked directly.
/// The Device page calls it right after a person switches Bluetooth on.
#[tauri::command]
pub async fn channel_probe_bluetooth(bluetooth: State<'_, Arc<BluetoothChannelService>>) -> Result<bool, String> {
    Ok(bluetooth.probe().await)
}

/// The test switches of the radios (debug builds only). `bluetooth` and
/// `network` each set one channel on or off; a missing value leaves that
/// channel as it is. Off makes the channel look gone to Fini and ends its
/// sessions; the real radios stay on.
#[cfg(feature = "devtools")]
#[tauri::command]
pub async fn channel_set_test_switch(
    network_channel: State<'_, Arc<crate::services::network_channel_service::NetworkChannelService>>,
    space_sync: State<'_, Arc<crate::services::space_sync_service::SpaceSyncService>>,
    bluetooth: Option<bool>,
    network: Option<bool>,
) -> Result<(), String> {
    use crate::models::channel::ChannelKind;
    if let Some(on) = network {
        network_channel.set_switch(on);
        if !on {
            space_sync.end_sessions_on(ChannelKind::Network);
        }
    }
    if let Some(on) = bluetooth {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            if !crate::services::bluetooth_channel_service::BluetoothAdapter::this_device().set_switch(on).await {
                return Err("this device has no usable Bluetooth adapter".to_string());
            }
            if !on {
                space_sync.end_sessions_on(ChannelKind::Bluetooth);
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let _ = on;
            return Err("Bluetooth is not available on this platform".to_string());
        }
    }
    Ok(())
}
