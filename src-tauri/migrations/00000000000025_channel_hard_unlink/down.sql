ALTER TABLE channels ADD COLUMN is_primary BOOLEAN NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX channels_one_primary_per_device ON channels (device_id) WHERE is_primary = 1;
DROP TABLE channel_unlink_notices;
ALTER TABLE channels ADD COLUMN unlinked_at TEXT;
