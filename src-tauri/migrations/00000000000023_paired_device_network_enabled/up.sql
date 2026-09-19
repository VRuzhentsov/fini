-- The device-pairing redesign gives every channel the same switch, so
-- Network gains the per-pair on/off Bluetooth has had since migration 19.
--
-- Defaults to 1, not 0, and the asymmetry with `bluetooth_enabled` is
-- deliberate: Bluetooth is off until a user adds it, whereas Network is the
-- channel every existing pair was formed over and is currently syncing on.
-- Defaulting it off would silently disconnect every pair in the field on
-- upgrade.
ALTER TABLE paired_devices
ADD COLUMN network_enabled BOOLEAN NOT NULL DEFAULT 1;
