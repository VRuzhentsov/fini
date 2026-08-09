-- No backfill here, deliberately: this column and the bluetooth_enabled
-- column it disambiguates from (migration 00000000000019) ship together,
-- unreleased -- no released build ever exposed a way to explicitly
-- disable Bluetooth (device_connection_set_bluetooth_transport_impl is
-- introduced on this same branch), so every pre-existing
-- bluetooth_enabled = 0 row is simply "never touched," not "explicitly
-- disabled." Backfilling it to true would opt every existing pair out of
-- the new automatic exchange flow by default on first upgrade.
ALTER TABLE paired_devices
ADD COLUMN bluetooth_disabled_by_user BOOLEAN NOT NULL DEFAULT 0;
