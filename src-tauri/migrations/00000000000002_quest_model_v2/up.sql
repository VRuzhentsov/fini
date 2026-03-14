ALTER TABLE quests ADD COLUMN energy TEXT NOT NULL DEFAULT 'medium';
ALTER TABLE quests ADD COLUMN due_time TEXT;
ALTER TABLE quests ADD COLUMN repeat_rule TEXT;
ALTER TABLE quests DROP COLUMN energy_required;
