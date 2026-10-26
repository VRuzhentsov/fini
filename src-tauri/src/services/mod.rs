#[cfg(all(any(feature = "ui-plane", test), target_os = "linux"))]
pub mod appimage_desktop;
#[cfg(target_os = "android")]
pub mod android_context;
pub mod backup;
#[cfg(all(target_os = "android", feature = "ui-plane"))]
pub mod ble_plugin;
#[cfg(any(target_os = "linux", target_os = "android"))]
#[path = "bluetooth-channel.service.rs"]
pub mod bluetooth_channel_service;
#[path = "channel.service.rs"]
pub mod channel_service;
pub mod checklist;
#[cfg(feature = "cli-plane")]
pub mod cli;
#[path = "device.service.rs"]
pub mod device_service;
#[path = "event-bus.service.rs"]
pub mod event_bus_service;
#[path = "network-channel.service.rs"]
pub mod network_channel_service;
#[path = "space-sync.service.rs"]
pub mod space_sync_service;
#[cfg(feature = "cli-plane")]
pub mod cli_update;
pub mod db;
#[cfg(feature = "ui-plane")]
pub mod desktop_update;
pub mod due_time;
pub mod migration_import;
#[cfg(test)]
mod migrations_tests;
#[cfg(any(feature = "ui-plane", test))]
pub mod notification;
pub mod quest;
#[cfg(any(feature = "ui-plane", test))]
pub mod reconciler;
pub mod reminder;
pub mod settings;
pub mod space;
pub mod update_recovery;

/// The Bluetooth channel on a system without `ble-gatt` (Windows, macOS):
/// the same name as the real one, so that the other services need no
/// platform branches. It reports that Bluetooth is not supported.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub mod bluetooth_channel_service {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use crate::models::channel::ChannelProblem;
    use crate::models::Identity;
    use crate::services::channel_service::{ChannelKind, ChannelService, DataLink, LinkHandlers};
    use crate::services::event_bus_service::EventBusService;

    pub struct BluetoothChannelService {
        db_path: PathBuf,
    }

    impl BluetoothChannelService {
        pub fn new(_identity: Identity, db_path: PathBuf, _bus: Arc<EventBusService>) -> Arc<Self> {
            Arc::new(Self { db_path })
        }

        pub fn has_session(&self, _peer_id: &str) -> bool {
            false
        }

        pub fn close_session(&self, _peer_id: &str) -> bool {
            false
        }

        pub fn refresh_advertising(&self) {}

        pub fn close_rejected_link(&self, _address: String, _device_id: Option<&str>) {}

        pub fn close_links_after_session(&self, _peer_id: &str) {}
    }

    #[async_trait::async_trait]
    impl ChannelService for BluetoothChannelService {
        fn kind(&self) -> ChannelKind {
            ChannelKind::Bluetooth
        }

        fn db_path(&self) -> &Path {
            &self.db_path
        }

        fn set_link_handlers(&self, _handlers: LinkHandlers) {}

        #[cfg(any(feature = "ui-plane", test))]
        fn available(&self) -> bool {
            false
        }

        /// No adapter on this platform, so there is no radio to be off.
        #[cfg(any(feature = "ui-plane", test))]
        async fn probe(&self) -> bool {
            true
        }

        #[cfg(any(feature = "ui-plane", test))]
        fn problem(&self) -> Option<ChannelProblem> {
            Some(ChannelProblem::BluetoothNotSupported)
        }
    }
}
