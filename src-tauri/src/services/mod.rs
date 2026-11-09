#[cfg(all(any(feature = "ui-plane", test), target_os = "linux"))]
pub mod appimage_desktop;
#[cfg(target_os = "android")]
pub mod android_context;
pub mod backup;
#[cfg(all(target_os = "android", feature = "ui-plane"))]
pub mod ble_plugin;
#[cfg(all(feature = "ui-plane", any(target_os = "linux", target_os = "android", target_os = "windows")))]
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
#[cfg(any(feature = "ui-plane", test))]
pub mod reconciler;
pub mod reminder;
pub mod settings;
pub mod space;
pub mod update_recovery;

/// The Bluetooth channel when `ble-gatt` is not compiled in (on macOS, and
/// without `ui-plane`, for example in the CLI): the same name as the real
/// one, so that the other services need no platform branches. It reports
/// that Bluetooth is not supported.
#[cfg(not(all(feature = "ui-plane", any(target_os = "linux", target_os = "android", target_os = "windows"))))]
pub mod bluetooth_channel_service {
    use std::sync::Arc;

    use diesel::SqliteConnection;

    use crate::models::Identity;
    use crate::services::channel_service::{ChannelKind, ChannelProblem, ChannelService, LinkHandlers};
    use crate::services::db::DbPool;
    use crate::services::event_bus_service::EventBus;

    pub struct BluetoothChannelService {
        pool: DbPool,
    }

    impl BluetoothChannelService {
        pub fn new(_identity: Identity, pool: DbPool, _bus: Arc<EventBus>) -> Arc<Self> {
            Arc::new(Self { pool })
        }

        pub fn refresh_advertising(&self, _conn: &mut SqliteConnection) {}

        pub fn connect_once_after_switch_on(&self, _peer_id: &str) {}

        pub fn note_refused(&self, _peer_id: &str) {}

        pub fn close_rejected_link(&self, _address: String, _device_id: Option<&str>) {}
    }

    #[async_trait::async_trait]
    impl ChannelService for BluetoothChannelService {
        fn kind(&self) -> ChannelKind {
            ChannelKind::Bluetooth
        }

        fn pool(&self) -> &DbPool {
            &self.pool
        }

        fn set_link_handlers(&self, _handlers: LinkHandlers) {}

        fn available(&self) -> bool {
            false
        }

        /// No adapter on this platform, so there is no radio to be off.
        #[cfg(any(feature = "ui-plane", test))]
        async fn probe(&self) -> bool {
            true
        }

        fn problem(&self) -> Option<ChannelProblem> {
            Some(ChannelProblem::BluetoothNotSupported)
        }
    }
}
