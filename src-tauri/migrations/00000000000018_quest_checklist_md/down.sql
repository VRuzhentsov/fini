DROP TABLE checklist_activity;

ALTER TABLE quest_series DROP COLUMN is_checklist;

ALTER TABLE quests DROP COLUMN checklist_base;
ALTER TABLE quests DROP COLUMN is_checklist;
