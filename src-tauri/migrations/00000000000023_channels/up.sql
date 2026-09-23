-- Channels become rows.
--
-- A channel is the configured connection between two devices, and until now
-- each one was a fixed set of columns on `paired_devices`: bluetooth_enabled,
-- bluetooth_address, bluetooth_last_verified_at, bluetooth_disabled_by_user,
-- network_enabled, preferred_transport. Adding a third channel meant another
-- migration and another four columns, and the two existing ones already
-- disagreed about their own defaults.
--
-- `channel_kinds` names the channels that exist; `channels` carries, per pair
-- and per kind, whether it is on, whether it is the primary, and any address
-- the channel learned. A new kind is an INSERT.

CREATE TABLE channel_kinds (
    code TEXT PRIMARY KEY NOT NULL
);

INSERT INTO channel_kinds (code) VALUES ('network'), ('bluetooth');

-- A row means *configured*. That is the third state the old booleans could
-- not express and had to fake with `bluetooth_disabled_by_user`:
--
--   no row           never set up -- the page offers to set it up
--   enabled = 0      set up, switched off
--   enabled = 1      on
--
-- `is_primary` rather than `primary`, which is a SQL keyword; the concept is
-- called "primary" everywhere else.
CREATE TABLE channels (
    device_id     TEXT NOT NULL REFERENCES paired_devices(peer_device_id) ON DELETE CASCADE,
    channel_kind  TEXT NOT NULL REFERENCES channel_kinds(code),
    enabled       BOOLEAN NOT NULL DEFAULT 0,
    is_primary    BOOLEAN NOT NULL DEFAULT 0,
    address       TEXT,
    configured_at TEXT NOT NULL,
    PRIMARY KEY (device_id, channel_kind)
);

CREATE UNIQUE INDEX channels_one_primary_per_device
    ON channels (device_id) WHERE is_primary = 1;

-- Every existing pair was formed over the network and is syncing on it, so
-- each gets a Network row, on.
INSERT INTO channels (device_id, channel_kind, enabled, is_primary, address, configured_at)
SELECT peer_device_id,
       'network',
       1,
       CASE WHEN preferred_transport = 'network' THEN 1 ELSE 0 END,
       NULL,
       paired_at
FROM paired_devices;

-- Bluetooth only where it was actually set up. `bluetooth_enabled` means it
-- is on; `bluetooth_disabled_by_user` means it was set up and switched off --
-- which is exactly what a row with `enabled = 0` now says, so the flag has
-- nothing left to record. A pair with neither gets no row, because it never
-- had a Bluetooth channel.
INSERT INTO channels (device_id, channel_kind, enabled, is_primary, address, configured_at)
SELECT peer_device_id,
       'bluetooth',
       CASE WHEN bluetooth_enabled = 1 THEN 1 ELSE 0 END,
       CASE WHEN preferred_transport = 'bluetooth' THEN 1 ELSE 0 END,
       bluetooth_address,
       paired_at
FROM paired_devices
WHERE bluetooth_enabled = 1 OR bluetooth_disabled_by_user = 1;

-- `bluetooth_last_verified_at` is not carried over. It was written in four
-- places and read in none since ADR-0006 removed the bond check it fed.
ALTER TABLE paired_devices DROP COLUMN bluetooth_enabled;
ALTER TABLE paired_devices DROP COLUMN bluetooth_address;
ALTER TABLE paired_devices DROP COLUMN bluetooth_last_verified_at;
ALTER TABLE paired_devices DROP COLUMN bluetooth_disabled_by_user;
ALTER TABLE paired_devices DROP COLUMN preferred_transport;
ALTER TABLE paired_devices DROP COLUMN preferred_transport_set_at;
