PRAGMA foreign_keys = OFF;

CREATE TABLE space_id_map (
    old_id INTEGER PRIMARY KEY NOT NULL,
    new_id TEXT NOT NULL UNIQUE
);

INSERT INTO space_id_map (old_id, new_id)
SELECT
    id,
    CASE
        WHEN lower(trim(name)) = 'personal' AND id = (
            SELECT MIN(id) FROM spaces WHERE lower(trim(name)) = 'personal'
        ) THEN '1'
        WHEN lower(trim(name)) = 'family' AND id = (
            SELECT MIN(id) FROM spaces WHERE lower(trim(name)) = 'family'
        ) THEN '2'
        WHEN lower(trim(name)) = 'work' AND id = (
            SELECT MIN(id) FROM spaces WHERE lower(trim(name)) = 'work'
        ) THEN '3'
        ELSE (
            lower(hex(randomblob(4))) || '-' ||
            lower(hex(randomblob(2))) || '-' ||
            '4' || substr(lower(hex(randomblob(2))), 2) || '-' ||
            substr('89ab', abs(random()) % 4 + 1, 1) || substr(lower(hex(randomblob(2))), 2) || '-' ||
            lower(hex(randomblob(6)))
        )
    END
FROM spaces;

CREATE TABLE spaces_new (
    id TEXT PRIMARY KEY NOT NULL DEFAULT (
        lower(hex(randomblob(4))) || '-' ||
        lower(hex(randomblob(2))) || '-' ||
        '4' || substr(lower(hex(randomblob(2))), 2) || '-' ||
        substr('89ab', abs(random()) % 4 + 1, 1) || substr(lower(hex(randomblob(2))), 2) || '-' ||
        lower(hex(randomblob(6)))
    ),
    name TEXT NOT NULL,
    item_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO spaces_new (id, name, item_order, created_at)
SELECT map.new_id, s.name, s.item_order, s.created_at
FROM spaces s
JOIN space_id_map map ON map.old_id = s.id;

INSERT INTO spaces_new (id, name, item_order, created_at)
SELECT
    '1',
    'Personal',
    COALESCE((SELECT MAX(item_order) + 1 FROM spaces_new), 0),
    datetime('now')
WHERE NOT EXISTS (SELECT 1 FROM spaces_new WHERE id = '1');

INSERT INTO spaces_new (id, name, item_order, created_at)
SELECT
    '2',
    'Family',
    COALESCE((SELECT MAX(item_order) + 1 FROM spaces_new), 0),
    datetime('now')
WHERE NOT EXISTS (SELECT 1 FROM spaces_new WHERE id = '2');

INSERT INTO spaces_new (id, name, item_order, created_at)
SELECT
    '3',
    'Work',
    COALESCE((SELECT MAX(item_order) + 1 FROM spaces_new), 0),
    datetime('now')
WHERE NOT EXISTS (SELECT 1 FROM spaces_new WHERE id = '3');

CREATE TABLE quests_new (
    id TEXT PRIMARY KEY NOT NULL DEFAULT (
        lower(hex(randomblob(4))) || '-' ||
        lower(hex(randomblob(2))) || '-' ||
        '4' || substr(lower(hex(randomblob(2))), 2) || '-' ||
        substr('89ab', abs(random()) % 4 + 1, 1) || substr(lower(hex(randomblob(2))), 2) || '-' ||
        lower(hex(randomblob(6)))
    ),
    space_id TEXT NOT NULL DEFAULT '1' REFERENCES spaces_new(id) ON DELETE SET DEFAULT,
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

INSERT INTO quests_new (
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
    (
        lower(hex(randomblob(4))) || '-' ||
        lower(hex(randomblob(2))) || '-' ||
        '4' || substr(lower(hex(randomblob(2))), 2) || '-' ||
        substr('89ab', abs(random()) % 4 + 1, 1) || substr(lower(hex(randomblob(2))), 2) || '-' ||
        lower(hex(randomblob(6)))
    ),
    COALESCE(map.new_id, '1'),
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
LEFT JOIN space_id_map map ON map.old_id = q.space_id;

DROP TABLE quests;
DROP TABLE spaces;

ALTER TABLE spaces_new RENAME TO spaces;
ALTER TABLE quests_new RENAME TO quests;

DROP TABLE space_id_map;

PRAGMA foreign_keys = ON;
