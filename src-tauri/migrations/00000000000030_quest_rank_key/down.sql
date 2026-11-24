-- `order_rank` becomes a decimal again: the number of quests with a lower
-- rank key, at most 100 (the limit of the decimal rank). Quests with the
-- same key get the same number. The order of the first 101 quests stays.

ALTER TABLE quests ADD COLUMN rank_number REAL NOT NULL DEFAULT 0;
UPDATE quests
SET rank_number = MIN(
    100.0,
    (SELECT COUNT(*) FROM quests AS q WHERE q.order_rank < quests.order_rank)
);
ALTER TABLE quests DROP COLUMN order_rank;
ALTER TABLE quests RENAME COLUMN rank_number TO order_rank;

UPDATE sync_outbox
SET payload = json_set(
    payload,
    '$.order_rank',
    COALESCE((SELECT q.order_rank FROM quests AS q WHERE q.id = sync_outbox.entity_id), 0.0)
)
WHERE entity_type = 'quest'
  AND payload IS NOT NULL
  AND json_valid(payload)
  AND json_type(payload, '$.order_rank') = 'text';
