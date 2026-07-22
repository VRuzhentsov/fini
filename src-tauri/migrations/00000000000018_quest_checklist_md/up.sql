-- Issue #128: a checklist quest reuses the existing `description` field as its storage — the
-- description textarea already is markdown-compatible, and a checklist is simply the same field
-- parsed/rendered as a task-list instead of prose. No dedicated checklist content column.
ALTER TABLE quests ADD COLUMN is_checklist BOOLEAN NOT NULL DEFAULT 0;
-- Device-local convergence bookkeeping for the per-item sync merge of `description` when
-- is_checklist=1 — the last description value both sides last agreed on. Never synced to peers.
ALTER TABLE quests ADD COLUMN checklist_base TEXT;

ALTER TABLE quest_series ADD COLUMN is_checklist BOOLEAN NOT NULL DEFAULT 0;

CREATE TABLE checklist_activity (
    id               TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    quest_id         TEXT NOT NULL,
    kind             TEXT NOT NULL,
    detail           TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    origin_device_id TEXT
);

CREATE INDEX idx_checklist_activity_quest_id ON checklist_activity (quest_id);
