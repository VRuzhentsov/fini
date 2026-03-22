PRAGMA foreign_keys = OFF;

CREATE TABLE space_id_map (
    new_id TEXT PRIMARY KEY NOT NULL,
    old_id INTEGER NOT NULL UNIQUE
);

INSERT INTO space_id_map (new_id, old_id)
VALUES
    ('1', 1),
    ('2', 2),
    ('3', 3);

INSERT INTO space_id_map (new_id, old_id)
SELECT
    s.id,
    (
        SELECT COUNT(*)
        FROM spaces s2
        WHERE
            s2.id NOT IN ('1', '2', '3') AND
            (
                s2.created_at < s.created_at OR
                (s2.created_at = s.created_at AND s2.id <= s.id)
            )
    ) + 3
FROM spaces s
WHERE s.id NOT IN ('1', '2', '3');

CREATE TABLE spaces_old (
    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    name TEXT NOT NULL,
    item_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO spaces_old (id, name, item_order, created_at)
SELECT map.old_id, s.name, s.item_order, s.created_at
FROM spaces s
JOIN space_id_map map ON map.new_id = s.id;

CREATE TABLE quests_old (
    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    space_id INTEGER NOT NULL DEFAULT 1 REFERENCES spaces_old(id) ON DELETE SET DEFAULT,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    energy TEXT NOT NULL DEFAULT 'medium',
    priority INTEGER NOT NULL DEFAULT 1,
    pinned BOOLEAN NOT NULL DEFAULT 0,
    due TEXT,
    due_time TEXT,
    repeat_rule TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO quests_old (
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
    COALESCE(map.old_id, 1),
    q.title,
    q.description,
    q.status,
    q.energy,
    q.priority,
    q.pinned,
    q.due,
    q.due_time,
    q.repeat_rule,
    q.completed_at,
    q.created_at,
    q.updated_at
FROM quests q
LEFT JOIN space_id_map map ON map.new_id = q.space_id;

DROP TABLE quests;
DROP TABLE spaces;

ALTER TABLE spaces_old RENAME TO spaces;
ALTER TABLE quests_old RENAME TO quests;

DROP TABLE space_id_map;

PRAGMA foreign_keys = ON;
