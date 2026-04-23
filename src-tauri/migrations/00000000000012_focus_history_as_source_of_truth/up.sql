-- Drop focus columns from quests (now owned exclusively by focus_history)
CREATE TABLE quests__new (
    id            TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    space_id      TEXT NOT NULL DEFAULT '1' REFERENCES spaces(id) ON DELETE SET DEFAULT,
    title         TEXT NOT NULL DEFAULT '',
    description   TEXT,
    status        TEXT NOT NULL DEFAULT 'active',
    energy        TEXT NOT NULL DEFAULT 'medium',
    priority      INTEGER NOT NULL DEFAULT 1,
    pinned        BOOLEAN NOT NULL DEFAULT 0,
    due           TEXT,
    due_time      TEXT,
    repeat_rule   TEXT,
    completed_at  TEXT,
    order_rank    REAL NOT NULL DEFAULT 0.0,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    series_id     TEXT REFERENCES quest_series(id) ON DELETE SET NULL,
    period_key    TEXT
);

INSERT INTO quests__new (id, space_id, title, description, status, energy, priority, pinned,
                          due, due_time, repeat_rule, completed_at,
                          order_rank, created_at, updated_at, series_id, period_key)
SELECT id, space_id, title, description, status, energy, priority, pinned,
       due, due_time, repeat_rule, completed_at,
       order_rank, created_at, updated_at, series_id, period_key
FROM quests;

DROP TABLE quests;
ALTER TABLE quests__new RENAME TO quests;

-- Drop device_id from focus_history (each device owns its own store; column adds no value)
CREATE TABLE focus_history__new (
    id         TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    quest_id   TEXT NOT NULL REFERENCES quests(id) ON DELETE CASCADE,
    space_id   TEXT NOT NULL,
    trigger    TEXT NOT NULL CHECK (trigger IN ('manual', 'reminder', 'restore', 'system')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

INSERT INTO focus_history__new (id, quest_id, space_id, trigger, created_at)
SELECT id, quest_id, space_id, trigger, created_at
FROM focus_history;

DROP TABLE focus_history;
ALTER TABLE focus_history__new RENAME TO focus_history;

-- Index to speed up resolver walk (newest-first)
CREATE INDEX idx_focus_history_created_at ON focus_history(created_at DESC, quest_id);
