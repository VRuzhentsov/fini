CREATE TEMP TABLE v21_quest_series_snapshot AS
SELECT id, space_id, title, description, repeat_rule, priority, energy, active, created_at, updated_at, is_checklist
FROM quest_series;
CREATE TEMP TABLE v21_quests_snapshot AS
SELECT id, space_id, title, description, status, energy, priority, pinned, due, due_time, repeat_rule,
       completed_at, order_rank, focus_enter_count, created_at, updated_at, series_id, period_key,
       is_checklist, checklist_base
FROM quests;
CREATE TEMP TABLE v21_series_reminder_templates_snapshot AS
SELECT id, series_id, kind, mm_offset, created_at
FROM series_reminder_templates;
CREATE TEMP TABLE v21_reminders_snapshot AS
SELECT id, quest_id, type, mm_offset, due_at_utc, created_at, scheduled_notification_id
FROM reminders;
CREATE TEMP TABLE v21_focus_history_snapshot AS
SELECT id, quest_id, space_id, trigger, created_at
FROM focus_history;

CREATE TABLE quest_series_replacement (
    id          TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    space_id    TEXT NOT NULL DEFAULT '1' REFERENCES spaces(id) ON DELETE SET DEFAULT,
    title       TEXT NOT NULL,
    description TEXT,
    repeat_rule TEXT NOT NULL,
    priority    INTEGER NOT NULL DEFAULT 1,
    energy      TEXT NOT NULL DEFAULT 'medium',
    active      BOOLEAN NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    is_checklist BOOLEAN NOT NULL DEFAULT 0
);

DROP TABLE quest_series;
ALTER TABLE quest_series_replacement RENAME TO quest_series;

INSERT INTO quest_series (
    id, space_id, title, description, repeat_rule, priority, energy, active, created_at, updated_at, is_checklist
)
SELECT
    id,
    space_id,
    title,
    description,
    repeat_rule,
    CASE priority
        WHEN 1 THEN 2
        WHEN 3 THEN 4
        ELSE 3
    END,
    CASE energy
        WHEN 1 THEN 'low'
        WHEN 3 THEN 'high'
        ELSE 'medium'
    END,
    active,
    created_at,
    updated_at,
    is_checklist
FROM v21_quest_series_snapshot;

CREATE TABLE quests_replacement (
    id          TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    space_id    TEXT NOT NULL DEFAULT '1' REFERENCES spaces(id) ON DELETE SET DEFAULT,
    title       TEXT NOT NULL,
    description TEXT,
    status      TEXT NOT NULL DEFAULT 'active',
    energy      TEXT NOT NULL DEFAULT 'medium',
    priority    INTEGER NOT NULL DEFAULT 1,
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

INSERT INTO quests_replacement (
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
    CASE energy
        WHEN 1 THEN 'low'
        WHEN 3 THEN 'high'
        ELSE 'medium'
    END,
    CASE priority
        WHEN 1 THEN 2
        WHEN 3 THEN 4
        ELSE 3
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
FROM v21_quests_snapshot;

DROP TABLE quests;
ALTER TABLE quests_replacement RENAME TO quests;
CREATE INDEX IF NOT EXISTS idx_quests_status ON quests (status);
CREATE INDEX IF NOT EXISTS idx_quests_series_period ON quests (series_id, period_key);

INSERT INTO series_reminder_templates (id, series_id, kind, mm_offset, created_at)
SELECT id, series_id, kind, mm_offset, created_at
FROM v21_series_reminder_templates_snapshot;
INSERT INTO reminders (id, quest_id, type, mm_offset, due_at_utc, created_at, scheduled_notification_id)
SELECT id, quest_id, type, mm_offset, due_at_utc, created_at, scheduled_notification_id
FROM v21_reminders_snapshot;
INSERT INTO focus_history (id, quest_id, space_id, trigger, created_at)
SELECT id, quest_id, space_id, trigger, created_at
FROM v21_focus_history_snapshot;

DROP TABLE v21_quest_series_snapshot;
DROP TABLE v21_quests_snapshot;
DROP TABLE v21_series_reminder_templates_snapshot;
DROP TABLE v21_reminders_snapshot;
DROP TABLE v21_focus_history_snapshot;
