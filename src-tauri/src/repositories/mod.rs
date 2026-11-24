use diesel::sqlite::SqliteConnection;

#[path = "channel.repository.rs"]
pub mod channel_repository;
#[path = "device.repository.rs"]
pub mod device_repository;
#[path = "outbox.repository.rs"]
pub mod outbox_repository;
#[path = "quest.repository.rs"]
pub mod quest_repository;
#[path = "reminder.repository.rs"]
pub mod reminder_repository;
#[path = "space-mapping.repository.rs"]
pub mod space_mapping_repository;
#[path = "space.repository.rs"]
pub mod space_repository;

pub use channel_repository::ChannelRepository;
pub use device_repository::DeviceRepository;
pub use outbox_repository::OutboxRepository;
pub use quest_repository::QuestRepository;
pub use reminder_repository::ReminderRepository;
pub use space_mapping_repository::SpaceMappingRepository;
pub use space_repository::SpaceRepository;

/// The interface of each repository (docs/db.md, "The parts"): it is made
/// on one connection, and it holds only that connection.
pub trait Repository<'a> {
    /// Makes the repository on `conn`.
    fn new(conn: &'a mut SqliteConnection) -> Self;
}
