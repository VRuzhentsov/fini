DROP TABLE channel_unlink_notices;
ALTER TABLE channels ADD COLUMN unlinked_at TEXT;
