//! The Tauri commands: the UI's RPC into the services (ADR-0010 D17). The
//! commands of each service are in one file.

#[path = "channel.commands.rs"]
pub mod channel_commands;
#[path = "device.commands.rs"]
pub mod device_commands;
#[path = "space-sync.commands.rs"]
pub mod space_sync_commands;
