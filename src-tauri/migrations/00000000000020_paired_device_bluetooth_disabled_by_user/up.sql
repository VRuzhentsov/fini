ALTER TABLE paired_devices
ADD COLUMN bluetooth_disabled_by_user BOOLEAN NOT NULL DEFAULT 0;
