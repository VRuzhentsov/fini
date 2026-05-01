CREATE TABLE settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO settings (key, value) VALUES ('theme', 'system');
