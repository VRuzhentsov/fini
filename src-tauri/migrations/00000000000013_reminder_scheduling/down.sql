DROP TABLE IF EXISTS series_reminder_templates;

-- SQLite does not support DROP COLUMN in older versions; recreate reminders without the column
CREATE TABLE reminders__old AS SELECT id, quest_id, type, mm_offset, due_at_utc, created_at FROM reminders;
DROP TABLE reminders;
ALTER TABLE reminders__old RENAME TO reminders;
