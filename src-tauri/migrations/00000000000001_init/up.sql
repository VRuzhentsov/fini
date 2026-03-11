CREATE TABLE spaces (
    id          INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    name        TEXT NOT NULL,
    item_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO spaces (name, item_order) VALUES ('Personal', 0);

CREATE TABLE quests (
    id              INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    space_id        INTEGER REFERENCES spaces(id) ON DELETE SET NULL,
    title           TEXT NOT NULL,
    description     TEXT,
    status          TEXT NOT NULL DEFAULT 'active',
    energy_required INTEGER,
    priority        INTEGER NOT NULL DEFAULT 1,
    pinned          BOOLEAN NOT NULL DEFAULT 0,
    due             TEXT,
    completed_at    TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

