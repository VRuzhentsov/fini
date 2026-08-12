PRAGMA foreign_keys = OFF;

CREATE TABLE quest_series_new (
    id          TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    space_id    TEXT NOT NULL DEFAULT '1' REFERENCES spaces(id) ON DELETE SET DEFAULT,
    title       TEXT NOT NULL,
    description TEXT,
    repeat_rule TEXT NOT NULL,
    priority    INTEGER NOT NULL DEFAULT 2 CHECK (priority IN (1, 2, 3)),
    energy      INTEGER NOT NULL DEFAULT 2 CHECK (energy IN (1, 2, 3)),
    active      BOOLEAN NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    is_checklist BOOLEAN NOT NULL DEFAULT 0
);

INSERT INTO quest_series_new (
    id, space_id, title, description, repeat_rule, priority, energy, active, created_at, updated_at, is_checklist
)
SELECT
    id,
    space_id,
    title,
    description,
    repeat_rule,
    CASE
        WHEN CAST(priority AS TEXT) IN ('2', 'low') THEN 1
        WHEN CAST(priority AS TEXT) IN ('4', 'urgent', 'high') THEN 3
        ELSE 2
    END,
    CASE
        WHEN CAST(energy AS TEXT) IN ('1', 'low', 'small') THEN 1
        WHEN CAST(energy AS TEXT) IN ('3', 'high', 'large') THEN 3
        ELSE 2
    END,
    active,
    created_at,
    updated_at,
    is_checklist
FROM quest_series;

DROP TABLE quest_series;
ALTER TABLE quest_series_new RENAME TO quest_series;

CREATE TABLE quests_new (
    id          TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    space_id    TEXT NOT NULL DEFAULT '1' REFERENCES spaces(id) ON DELETE SET DEFAULT,
    title       TEXT NOT NULL,
    description TEXT,
    status      TEXT NOT NULL DEFAULT 'active',
    energy      INTEGER NOT NULL DEFAULT 2 CHECK (energy IN (1, 2, 3)),
    priority    INTEGER NOT NULL DEFAULT 2 CHECK (priority IN (1, 2, 3)),
    pinned      BOOLEAN NOT NULL DEFAULT 0,
    due         TEXT,
    due_time    TEXT,
    repeat_rule TEXT,
    completed_at TEXT,
    order_rank REAL NOT NULL DEFAULT 0,
    focus_enter_count INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    series_id   TEXT REFERENCES quest_series(id) ON DELETE CASCADE,
    period_key  TEXT,
    is_checklist BOOLEAN NOT NULL DEFAULT 0,
    checklist_base TEXT
);

INSERT INTO quests_new (
    id, space_id, title, description, status, energy, priority, pinned, due, due_time, repeat_rule,
    completed_at, order_rank, focus_enter_count, created_at, updated_at, series_id, period_key,
    is_checklist, checklist_base
)
SELECT
    id,
    space_id,
    title,
    description,
    status,
    CASE
        WHEN CAST(energy AS TEXT) IN ('1', 'low', 'small') THEN 1
        WHEN CAST(energy AS TEXT) IN ('3', 'high', 'large') THEN 3
        ELSE 2
    END,
    CASE
        WHEN CAST(priority AS TEXT) IN ('2', 'low') THEN 1
        WHEN CAST(priority AS TEXT) IN ('4', 'urgent', 'high') THEN 3
        ELSE 2
    END,
    pinned,
    due,
    due_time,
    repeat_rule,
    completed_at,
    order_rank,
    focus_enter_count,
    created_at,
    updated_at,
    series_id,
    period_key,
    is_checklist,
    checklist_base
FROM quests;

DROP TABLE quests;
ALTER TABLE quests_new RENAME TO quests;
CREATE INDEX IF NOT EXISTS idx_quests_status ON quests (status);
CREATE INDEX IF NOT EXISTS idx_quests_series_period ON quests (series_id, period_key);

PRAGMA foreign_keys = ON;
