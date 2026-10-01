-- ADR-0008 D14/D15: a channel is `None` (no row), `Off` or `On`.
--
-- Unlinking deletes the row again. The tombstone (`unlinked_at`, migration
-- 24) existed so a peer could not recreate a channel this person removed;
-- under ADR-0008 a channel is only ever created by a mutual init, which
-- needs both people searching, so no peer can recreate one on its own.
-- Rows that were already unlinked are removed: they meant "absent".
DELETE FROM channels WHERE unlinked_at IS NOT NULL;
ALTER TABLE channels DROP COLUMN unlinked_at;

-- Unlinking also tells the peer, as a pushed frame (ADR-0008 D14); nothing
-- about it is stored.

-- `is_primary` stays: the person's choice of which channel carries the
-- pair's traffic (ADR-0007) outlives held sessions.
