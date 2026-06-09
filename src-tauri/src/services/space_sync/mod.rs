mod commands;
mod merge;
pub(crate) mod outbox;
mod replay;
pub mod types;
pub(crate) mod ws_client;
#[cfg(any(feature = "ui-plane", test))]
pub(crate) mod ws_server;
pub(crate) mod ws_session;

#[cfg(any(feature = "ui-plane", test))]
pub use commands::{
    space_sync_apply_remote_mappings, space_sync_list_mappings,
    space_sync_resolve_custom_space_mapping, space_sync_status, space_sync_tick,
    space_sync_update_mappings,
};
#[cfg(feature = "cli-plane")]
pub use commands::{
    space_sync_apply_remote_mappings_impl, space_sync_list_mappings_impl,
    space_sync_resolve_custom_space_mapping_impl, space_sync_status_impl, space_sync_tick_impl,
    space_sync_update_mappings_impl, SpaceResolutionMode,
};
#[cfg(any(feature = "ui-plane", test))]
pub use ws_server::run_ws_server;
