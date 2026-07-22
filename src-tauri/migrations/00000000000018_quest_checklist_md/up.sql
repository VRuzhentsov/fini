ALTER TABLE quests ADD COLUMN checklist_md TEXT;
ALTER TABLE quests ADD COLUMN checklist_md_base TEXT;
ALTER TABLE quest_series ADD COLUMN checklist_template_md TEXT;

CREATE TABLE checklist_activity (
    id               TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    quest_id         TEXT NOT NULL,
    kind             TEXT NOT NULL,
    detail           TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    origin_device_id TEXT
);

CREATE INDEX idx_checklist_activity_quest_id ON checklist_activity (quest_id);
