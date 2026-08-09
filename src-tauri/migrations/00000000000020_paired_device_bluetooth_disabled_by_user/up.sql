ALTER TABLE paired_devices
ADD COLUMN bluetooth_disabled_by_user BOOLEAN NOT NULL DEFAULT 0;

-- Conservative backfill for rows that already existed before this column
-- did: the previous schema had no way to distinguish "never enabled" from
-- "the user explicitly disabled it" -- both cleared to the same
-- bluetooth_enabled = 0 / bluetooth_address IS NULL state. Treat every
-- pre-existing not-currently-enabled row as an opt-out rather than risk
-- the peer's next self-report silently re-enabling a choice the user
-- actually made under the old schema. A never-touched pair only costs the
-- user one manual re-enable via the settings toggle; silently reversing a
-- real disable is the worse failure mode. Rows created after this
-- migration runs are unaffected -- they get the column's own default (0).
UPDATE paired_devices SET bluetooth_disabled_by_user = 1 WHERE bluetooth_enabled = 0;
