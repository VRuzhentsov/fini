-- Restore set_focus_at + reminder_triggered_at to quests
ALTER TABLE quests ADD COLUMN set_focus_at TEXT;
ALTER TABLE quests ADD COLUMN reminder_triggered_at TEXT;

-- Restore device_id to focus_history (with empty-string default since old data is gone)
ALTER TABLE focus_history ADD COLUMN device_id TEXT NOT NULL DEFAULT '';

DROP INDEX IF EXISTS idx_focus_history_created_at;
