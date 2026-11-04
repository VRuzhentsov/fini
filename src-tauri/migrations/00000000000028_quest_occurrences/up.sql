-- ADR-0012: a repeating quest is one quest. Its occurrences are computed
-- from its rule; `quest_occurrences` keeps only the days a person closed or
-- changed. The old series data is converted (ADR-0012 D11).
--
-- `quests.series_id` has a foreign key, which SQLite's DROP COLUMN refuses,
-- so `quests` is rebuilt by SQLite's generalized ALTER TABLE procedure:
-- foreign keys off, one transaction, `foreign_key_check`, foreign keys on.
PRAGMA foreign_keys = OFF;
BEGIN;

CREATE TABLE quest_occurrences (
    quest_id       TEXT NOT NULL REFERENCES quests(id) ON DELETE CASCADE,
    period_key     TEXT NOT NULL,
    status         TEXT CHECK (status IN ('completed', 'abandoned')),
    closed_at      TEXT,
    title          TEXT,
    description    TEXT,
    energy         INTEGER CHECK (energy IN (1, 2, 3)),
    priority       INTEGER CHECK (priority IN (1, 2, 3)),
    due_time       TEXT,
    checklist_base TEXT,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (quest_id, period_key)
);

-- Every old occurrence row, and the one quest its series becomes (its id
-- is the series id, the same on every device).
CREATE TEMP TABLE occurrence_rows AS
SELECT q.id AS occurrence_id, q.series_id AS quest_id
FROM quests q
WHERE q.series_id IS NOT NULL;

-- The row each series' quest takes its fields from: its open occurrence
-- (at most one was generated), else its newest one. A new occurrence was a
-- copy of the one before, so this row holds the person's last edits; the
-- series row can be older. Only a checklist's items come from the series
-- template, because the open row holds that day's ticks.
CREATE TEMP TABLE current_rows AS
SELECT s.id AS quest_id,
       COALESCE(
           (SELECT q.id FROM quests q WHERE q.series_id = s.id AND q.status = 'active'
            ORDER BY q.period_key LIMIT 1),
           (SELECT q.id FROM quests q WHERE q.series_id = s.id
            ORDER BY q.period_key DESC LIMIT 1)
       ) AS row_id,
       EXISTS (SELECT 1 FROM quests q WHERE q.series_id = s.id AND q.status = 'active') AS has_open
FROM quest_series s;

-- The one quest of each series. Its date is the open occurrence's date; a
-- series with no open occurrence ended.
CREATE TEMP TABLE series_quests AS
SELECT s.id,
       s.space_id,
       COALESCE(r.title, s.title) AS title,
       CASE WHEN s.is_checklist THEN s.description ELSE COALESCE(r.description, s.description) END AS description,
       CASE WHEN s.active AND c.has_open THEN 'active' ELSE 'completed' END AS status,
       COALESCE(r.period_key, r.due) AS due,
       r.due_time,
       s.repeat_rule,
       CASE WHEN s.active AND c.has_open THEN NULL
            ELSE (SELECT MAX(COALESCE(q.completed_at, q.updated_at)) FROM quests q WHERE q.series_id = s.id) END AS completed_at,
       COALESCE(r.order_rank, 0.0) AS order_rank,
       COALESCE((SELECT SUM(q.focus_enter_count) FROM quests q WHERE q.series_id = s.id), 0) AS focus_enter_count,
       s.created_at,
       s.updated_at,
       s.is_checklist,
       COALESCE(r.energy, s.energy) AS energy,
       COALESCE(r.priority, s.priority) AS priority
FROM quest_series s
JOIN current_rows c ON c.quest_id = s.id
LEFT JOIN quests r ON r.id = c.row_id;

-- Closed occurrences become marks. A field is stored only where the day
-- differs from its quest (an empty field takes the quest's value). The
-- description of a checklist day is always that day's copy (ADR-0012 D5),
-- so a later template edit does not change a closed day.
INSERT OR IGNORE INTO quest_occurrences
    (quest_id, period_key, status, closed_at, title, description, energy, priority, due_time,
     checklist_base, created_at, updated_at)
SELECT q.series_id,
       COALESCE(q.period_key, q.due, substr(q.created_at, 1, 10)),
       q.status,
       COALESCE(q.completed_at, q.updated_at),
       NULLIF(q.title, sq.title),
       CASE WHEN q.description IS sq.description AND NOT sq.is_checklist THEN NULL ELSE q.description END,
       NULLIF(q.energy, sq.energy),
       NULLIF(q.priority, sq.priority),
       CASE WHEN q.due_time IS sq.due_time THEN NULL ELSE q.due_time END,
       q.checklist_base,
       q.created_at,
       q.updated_at
FROM quests q
JOIN series_quests sq ON sq.id = q.series_id
WHERE q.status IN ('completed', 'abandoned')
ORDER BY q.updated_at DESC;

-- An open occurrence becomes a mark only when it differs from its quest
-- (its own ticks).
INSERT OR IGNORE INTO quest_occurrences
    (quest_id, period_key, status, closed_at, description, checklist_base, created_at, updated_at)
SELECT q.series_id, q.period_key, NULL, NULL, q.description, q.checklist_base, q.created_at, q.updated_at
FROM quests q
JOIN series_quests sq ON sq.id = q.series_id
WHERE q.status = 'active'
  AND q.description IS NOT sq.description;

-- Focus history and checklist activity point at the quest now.
UPDATE focus_history
SET quest_id = (SELECT quest_id FROM occurrence_rows WHERE occurrence_id = focus_history.quest_id)
WHERE quest_id IN (SELECT occurrence_id FROM occurrence_rows);

UPDATE checklist_activity
SET quest_id = (SELECT quest_id FROM occurrence_rows WHERE occurrence_id = checklist_activity.quest_id)
WHERE quest_id IN (SELECT occurrence_id FROM occurrence_rows);

-- Reminders belong to the quest and keep only their rule (ADR-0012 D9).
DELETE FROM reminders WHERE quest_id IN (SELECT occurrence_id FROM occurrence_rows);
INSERT INTO reminders (quest_id, type, mm_offset, due_at_utc, created_at)
SELECT t.series_id, t.kind, t.mm_offset, NULL, t.created_at
FROM series_reminder_templates t;

-- The new `quests`, without the series link.
CREATE TABLE quests__new (
    id                TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    space_id          TEXT NOT NULL DEFAULT '1' REFERENCES spaces(id) ON DELETE SET DEFAULT,
    title             TEXT NOT NULL DEFAULT '',
    description       TEXT,
    status            TEXT NOT NULL DEFAULT 'active',
    due               TEXT,
    due_time          TEXT,
    repeat_rule       TEXT,
    completed_at      TEXT,
    order_rank        REAL NOT NULL DEFAULT 0.0,
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    focus_enter_count INTEGER NOT NULL DEFAULT 0,
    is_checklist      BOOLEAN NOT NULL DEFAULT 0,
    checklist_base    TEXT,
    energy            INTEGER NOT NULL DEFAULT 2 CHECK (energy IN (1, 2, 3)),
    priority          INTEGER NOT NULL DEFAULT 2 CHECK (priority IN (1, 2, 3))
);

-- One-time quests as they are.
INSERT INTO quests__new
    (id, space_id, title, description, status, due, due_time, repeat_rule, completed_at, order_rank,
     created_at, updated_at, focus_enter_count, is_checklist, checklist_base, energy, priority)
SELECT id, space_id, title, description, status, due, due_time, repeat_rule, completed_at, order_rank,
       created_at, updated_at, focus_enter_count, is_checklist, checklist_base, energy, priority
FROM quests
WHERE series_id IS NULL;

-- One quest for each series.
INSERT INTO quests__new
    (id, space_id, title, description, status, due, due_time, repeat_rule, completed_at, order_rank,
     created_at, updated_at, focus_enter_count, is_checklist, checklist_base, energy, priority)
SELECT id, space_id, title, description, status, due, due_time, repeat_rule, completed_at, order_rank,
       created_at, updated_at, focus_enter_count, is_checklist, NULL, energy, priority
FROM series_quests;

DROP TABLE quests;
ALTER TABLE quests__new RENAME TO quests;

DROP TABLE series_reminder_templates;
DROP TABLE quest_series;

-- Rows that pointed at an old occurrence that is gone (no series quest
-- took it over) go too.
DELETE FROM focus_history WHERE quest_id NOT IN (SELECT id FROM quests);
DELETE FROM reminders WHERE quest_id NOT IN (SELECT id FROM quests);
DELETE FROM checklist_activity WHERE quest_id NOT IN (SELECT id FROM quests);

-- Sync: events of the old shape are not sent; every mapped space is sent
-- again in the new shape (bootstrap), with ids that are the same on every
-- device.
DELETE FROM sync_outbox WHERE entity_type = 'quest_series';
DELETE FROM sync_outbox
WHERE entity_type = 'quest'
  AND entity_id IN (SELECT occurrence_id FROM occurrence_rows);
UPDATE pair_space_mappings SET last_synced_at = NULL WHERE end_of_sync_at IS NULL;

DROP TABLE occurrence_rows;
DROP TABLE current_rows;
DROP TABLE series_quests;

PRAGMA foreign_key_check;
COMMIT;
PRAGMA foreign_keys = ON;
