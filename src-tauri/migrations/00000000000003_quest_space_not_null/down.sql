PRAGMA foreign_keys = OFF;

CREATE TABLE quests_old (
    id           INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    space_id     INTEGER REFERENCES spaces(id) ON DELETE SET NULL,
    title        TEXT NOT NULL,
    description  TEXT,
    status       TEXT NOT NULL DEFAULT 'active',
    energy       TEXT NOT NULL DEFAULT 'medium',
    priority     INTEGER NOT NULL DEFAULT 1,
    pinned       BOOLEAN NOT NULL DEFAULT 0,
    due          TEXT,
    due_time     TEXT,
    repeat_rule  TEXT,
    completed_at TEXT,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO quests_old (
    id,
    space_id,
    title,
    description,
    status,
    energy,
    priority,
    pinned,
    due,
    due_time,
    repeat_rule,
    completed_at,
    created_at,
    updated_at
)
SELECT
    id,
    space_id,
    title,
    description,
    status,
    energy,
    priority,
    pinned,
    due,
    due_time,
    repeat_rule,
    completed_at,
    created_at,
    updated_at
FROM quests;

DROP TABLE quests;
ALTER TABLE quests_old RENAME TO quests;

PRAGMA foreign_keys = ON;
