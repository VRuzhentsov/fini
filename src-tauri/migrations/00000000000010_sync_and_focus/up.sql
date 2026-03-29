-- Rename set_main_at -> set_focus_at
ALTER TABLE quests RENAME COLUMN set_main_at TO set_focus_at;

-- Paired devices (source of truth moves from localStorage to SQLite)
CREATE TABLE paired_devices (
    peer_device_id TEXT PRIMARY KEY NOT NULL,
    display_name   TEXT NOT NULL,
    paired_at      TEXT NOT NULL,
    last_seen_at   TEXT,
    pair_state     TEXT NOT NULL DEFAULT 'paired'
);

-- Pair-level symmetric space mappings
CREATE TABLE pair_space_mappings (
    peer_device_id TEXT NOT NULL REFERENCES paired_devices(peer_device_id) ON DELETE CASCADE,
    space_id       TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    enabled_at     TEXT NOT NULL,
    PRIMARY KEY (peer_device_id, space_id)
);

-- Focus history (owner-scoped, replaces quest-row focus fields long-term)
CREATE TABLE focus_history (
    id         TEXT PRIMARY KEY NOT NULL DEFAULT (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6)))),
    device_id  TEXT NOT NULL,
    quest_id   TEXT NOT NULL REFERENCES quests(id) ON DELETE CASCADE,
    space_id   TEXT NOT NULL,
    trigger    TEXT NOT NULL CHECK (trigger IN ('manual', 'reminder', 'restore', 'system')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX idx_focus_history_quest_id ON focus_history(quest_id);
CREATE INDEX idx_focus_history_device_id ON focus_history(device_id);

-- Sync outbox (durable event queue)
CREATE TABLE sync_outbox (
    event_id         TEXT PRIMARY KEY NOT NULL,
    correlation_id   TEXT NOT NULL,
    origin_device_id TEXT NOT NULL,
    entity_type      TEXT NOT NULL,
    entity_id        TEXT NOT NULL,
    space_id         TEXT NOT NULL,
    op_type          TEXT NOT NULL CHECK (op_type IN ('upsert', 'delete')),
    payload          TEXT,
    updated_at       TEXT NOT NULL,
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX idx_sync_outbox_space_id ON sync_outbox(space_id);
CREATE INDEX idx_sync_outbox_created_at ON sync_outbox(created_at);

-- Per-peer ACK tracking
CREATE TABLE sync_acks (
    peer_device_id TEXT NOT NULL,
    event_id       TEXT NOT NULL,
    acked_at       TEXT NOT NULL,
    PRIMARY KEY (peer_device_id, event_id)
);

-- Event dedupe (for relayed events)
CREATE TABLE sync_seen (
    event_id    TEXT PRIMARY KEY NOT NULL,
    received_at TEXT NOT NULL
);

-- Tombstones (30-day retention)
CREATE TABLE tombstones (
    entity_type TEXT NOT NULL,
    entity_id   TEXT NOT NULL,
    space_id    TEXT NOT NULL,
    deleted_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (entity_type, entity_id)
);

CREATE INDEX idx_tombstones_deleted_at ON tombstones(deleted_at);
