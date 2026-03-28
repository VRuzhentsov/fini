CREATE TABLE reminders (
    id          TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    quest_id    TEXT NOT NULL REFERENCES quests(id) ON DELETE CASCADE,
    type        TEXT NOT NULL CHECK (type IN ('relative', 'absolute')),
    mm_offset   INTEGER,
    due_at_utc  TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX idx_reminders_quest_id ON reminders(quest_id);
CREATE INDEX idx_reminders_due_at_utc ON reminders(due_at_utc);
