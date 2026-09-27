-- ADR-0008 D14/D15: a channel is `None` (no row), `Off` or `On`.
--
-- Unlinking deletes the row again. The tombstone (`unlinked_at`, migration
-- 24) existed so a peer could not recreate a channel this person removed;
-- under ADR-0008 a channel is only ever created by a mutual init, which
-- needs both people searching, so no peer can recreate one on its own.
-- Rows that were already unlinked are removed: they meant "absent".
DELETE FROM channels WHERE unlinked_at IS NOT NULL;
ALTER TABLE channels DROP COLUMN unlinked_at;

-- Unlinking also tells the peer, which then removes its own row. The notice
-- waits here until the peer acknowledges it, so it survives restarts and a
-- peer that is out of reach for a while.
CREATE TABLE channel_unlink_notices (
    device_id    TEXT NOT NULL REFERENCES paired_devices(peer_device_id) ON DELETE CASCADE,
    channel_kind TEXT NOT NULL REFERENCES channel_kinds(code),
    created_at   TEXT NOT NULL,
    PRIMARY KEY (device_id, channel_kind)
);

-- The primary-channel pin goes too: an exchange uses whichever channel is
-- On and reaches the peer, Network first (ADR-0008 D10, D19 has no pin).
DROP INDEX channels_one_primary_per_device;
ALTER TABLE channels DROP COLUMN is_primary;
