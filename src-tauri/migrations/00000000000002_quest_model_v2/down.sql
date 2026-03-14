ALTER TABLE quests ADD COLUMN energy_required INTEGER;
ALTER TABLE quests DROP COLUMN energy;
ALTER TABLE quests DROP COLUMN due_time;
ALTER TABLE quests DROP COLUMN repeat_rule;
