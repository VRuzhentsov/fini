-- `order_rank` becomes a rank key (src-tauri/src/models/rank.rs): text that
-- orders by byte comparison, with a key between any two keys, so a move
-- writes only the moved quest.
--
-- Each decimal value becomes the key `c` + 3 base-62 digits of
-- round((value + 100) * 1000), with the value clamped to [-100, 100]
-- (`Rank::from_legacy`). The key comes from the value alone, so two devices
-- that sync give the same key to the same quest, and the order stays the
-- same on both. Equal values give equal keys; the list order then uses
-- Priority, `created_at` and the id, as before.

CREATE TEMP TABLE quest_rank_keys AS
WITH positions AS (
    SELECT id,
           CAST(ROUND((MIN(MAX(order_rank, -100.0), 100.0) + 100.0) * 1000.0) AS INTEGER) AS position
    FROM quests
),
digits AS (
    SELECT '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz' AS digit
)
SELECT id,
       'c' || substr(digit, position / 3844 + 1, 1)
           || substr(digit, position / 62 % 62 + 1, 1)
           || substr(digit, position % 62 + 1, 1) AS rank_key
FROM positions, digits;

ALTER TABLE quests ADD COLUMN rank_key TEXT NOT NULL DEFAULT 'a0';
UPDATE quests
SET rank_key = (SELECT k.rank_key FROM quest_rank_keys AS k WHERE k.id = quests.id);
ALTER TABLE quests DROP COLUMN order_rank;
ALTER TABLE quests RENAME COLUMN rank_key TO order_rank;
DROP TABLE quest_rank_keys;

-- The quest events that wait in the outbox get the same key, so that a peer
-- on this version can read them (`order_rank` is text on the wire too).
CREATE TEMP TABLE outbox_rank_keys AS
WITH positions AS (
    SELECT event_id,
           CAST(ROUND((MIN(MAX(json_extract(payload, '$.order_rank'), -100.0), 100.0) + 100.0) * 1000.0) AS INTEGER) AS position
    FROM sync_outbox
    WHERE entity_type = 'quest'
      AND payload IS NOT NULL
      AND json_valid(payload)
      AND json_type(payload, '$.order_rank') IN ('integer', 'real')
),
digits AS (
    SELECT '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz' AS digit
)
SELECT event_id,
       'c' || substr(digit, position / 3844 + 1, 1)
           || substr(digit, position / 62 % 62 + 1, 1)
           || substr(digit, position % 62 + 1, 1) AS rank_key
FROM positions, digits;

UPDATE sync_outbox
SET payload = json_set(
    payload,
    '$.order_rank',
    (SELECT k.rank_key FROM outbox_rank_keys AS k WHERE k.event_id = sync_outbox.event_id)
)
WHERE event_id IN (SELECT event_id FROM outbox_rank_keys);
DROP TABLE outbox_rank_keys;
