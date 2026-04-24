-- Add OS scheduler handle to reminders (platform-specific notification ID)
ALTER TABLE reminders ADD COLUMN scheduled_notification_id TEXT;

-- Reminder templates for repeating series (materialized into reminders on each occurrence)
CREATE TABLE series_reminder_templates (
    id          TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    series_id   TEXT NOT NULL REFERENCES quest_series(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('relative', 'absolute')),
    mm_offset   INTEGER,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX idx_series_reminder_templates_series_id ON series_reminder_templates(series_id);
