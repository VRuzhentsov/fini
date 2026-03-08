CREATE TABLE quests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    energy_required INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE quest_steps (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    quest_id INTEGER NOT NULL REFERENCES quests(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    done INTEGER NOT NULL DEFAULT 0,
    step_order INTEGER NOT NULL DEFAULT 0
);

PRAGMA foreign_keys = ON;
