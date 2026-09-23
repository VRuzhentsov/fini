ALTER TABLE paired_devices ADD COLUMN bluetooth_enabled BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE paired_devices ADD COLUMN bluetooth_address TEXT;
ALTER TABLE paired_devices ADD COLUMN bluetooth_last_verified_at TEXT;
ALTER TABLE paired_devices ADD COLUMN bluetooth_disabled_by_user BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE paired_devices ADD COLUMN preferred_transport TEXT;
ALTER TABLE paired_devices ADD COLUMN preferred_transport_set_at TEXT;

UPDATE paired_devices
SET bluetooth_enabled = COALESCE(
        (SELECT enabled FROM channels
          WHERE channels.device_id = paired_devices.peer_device_id
            AND channels.channel_kind = 'bluetooth'), 0),
    bluetooth_address = (SELECT address FROM channels
          WHERE channels.device_id = paired_devices.peer_device_id
            AND channels.channel_kind = 'bluetooth'),
    bluetooth_disabled_by_user = COALESCE(
        (SELECT CASE WHEN enabled = 0 THEN 1 ELSE 0 END FROM channels
          WHERE channels.device_id = paired_devices.peer_device_id
            AND channels.channel_kind = 'bluetooth'), 0),
    preferred_transport = (SELECT channel_kind FROM channels
          WHERE channels.device_id = paired_devices.peer_device_id
            AND channels.is_primary = 1);

DROP INDEX channels_one_primary_per_device;
DROP TABLE channels;
DROP TABLE channel_kinds;
