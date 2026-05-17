CREATE TABLE notification_snoozes (
    reminder_id TEXT PRIMARY KEY NOT NULL,
    fire_at_utc TEXT NOT NULL,
    created_at  TEXT NOT NULL
);
