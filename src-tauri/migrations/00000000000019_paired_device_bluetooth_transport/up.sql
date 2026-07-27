ALTER TABLE paired_devices
ADD COLUMN bluetooth_enabled BOOLEAN NOT NULL DEFAULT 0;

ALTER TABLE paired_devices
ADD COLUMN bluetooth_address TEXT;

ALTER TABLE paired_devices
ADD COLUMN bluetooth_last_verified_at TEXT;
