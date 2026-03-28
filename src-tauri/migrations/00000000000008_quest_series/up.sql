-- Quest series: template records for repeating quests
CREATE TABLE quest_series (
    id          TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    space_id    TEXT NOT NULL DEFAULT '1' REFERENCES spaces(id) ON DELETE SET DEFAULT,
    title       TEXT NOT NULL,
    description TEXT,
    repeat_rule TEXT NOT NULL,
    priority    INTEGER NOT NULL DEFAULT 1,
    energy      TEXT NOT NULL DEFAULT 'medium',
    active      BOOLEAN NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Link quests to their parent series (null for standalone quests)
ALTER TABLE quests ADD COLUMN series_id TEXT REFERENCES quest_series(id) ON DELETE CASCADE;
ALTER TABLE quests ADD COLUMN period_key TEXT;

-- Migrate existing repeating quests into series
-- For each quest with a repeat_rule, create a series and link the quest to it
INSERT INTO quest_series (id, space_id, title, description, repeat_rule, priority, energy, created_at, updated_at)
SELECT
    id,
    space_id,
    title,
    description,
    repeat_rule,
    priority,
    energy,
    created_at,
    updated_at
FROM quests
WHERE repeat_rule IS NOT NULL AND repeat_rule != '';

-- Link existing repeating quests to their newly created series
UPDATE quests
SET series_id = id,
    period_key = COALESCE(due, substr(created_at, 1, 10))
WHERE repeat_rule IS NOT NULL AND repeat_rule != '';
