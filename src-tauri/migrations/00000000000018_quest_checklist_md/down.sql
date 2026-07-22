DROP TABLE checklist_activity;

ALTER TABLE quest_series DROP COLUMN checklist_template_md;
ALTER TABLE quests DROP COLUMN checklist_md_base;
ALTER TABLE quests DROP COLUMN checklist_md;
