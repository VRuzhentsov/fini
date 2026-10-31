-- The shape before ADR-0012. The data conversion is one way: repeating
-- quests stay single rows with their rule, and occurrence marks are lost.
CREATE TABLE quest_series (
    id          TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    space_id    TEXT NOT NULL DEFAULT '1' REFERENCES spaces(id) ON DELETE SET DEFAULT,
    title       TEXT NOT NULL,
    description TEXT,
    repeat_rule TEXT NOT NULL,
    active      BOOLEAN NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    is_checklist BOOLEAN NOT NULL DEFAULT 0,
    priority    INTEGER NOT NULL DEFAULT 2 CHECK (priority IN (1, 2, 3)),
    energy      INTEGER NOT NULL DEFAULT 2 CHECK (energy IN (1, 2, 3))
);
CREATE TABLE series_reminder_templates (
    id         TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    series_id  TEXT NOT NULL REFERENCES quest_series(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('relative', 'absolute')),
    mm_offset  INTEGER,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX idx_series_reminder_templates_series_id ON series_reminder_templates(series_id);
ALTER TABLE quests ADD COLUMN series_id TEXT REFERENCES quest_series(id) ON DELETE SET NULL;
ALTER TABLE quests ADD COLUMN period_key TEXT;
DROP TABLE quest_occurrences;
