PRAGMA foreign_keys = OFF;

-- Repair buggy v4 migration outcome where both built-in slots ended up named Work.
UPDATE spaces
SET name = 'Family'
WHERE id = '2'
  AND lower(trim(name)) = 'work'
  AND EXISTS (
      SELECT 1 FROM spaces s3 WHERE s3.id = '3' AND lower(trim(s3.name)) = 'work'
  )
  AND NOT EXISTS (
      SELECT 1 FROM spaces sf WHERE lower(trim(sf.name)) = 'family'
  );

-- Ensure built-in ids exist.
INSERT INTO spaces (id, name, item_order, created_at)
SELECT
    '1',
    'Personal',
    COALESCE((SELECT MAX(item_order) + 1 FROM spaces), 0),
    datetime('now')
WHERE NOT EXISTS (SELECT 1 FROM spaces WHERE id = '1');

INSERT INTO spaces (id, name, item_order, created_at)
SELECT
    '2',
    'Family',
    COALESCE((SELECT MAX(item_order) + 1 FROM spaces), 0),
    datetime('now')
WHERE NOT EXISTS (SELECT 1 FROM spaces WHERE id = '2');

INSERT INTO spaces (id, name, item_order, created_at)
SELECT
    '3',
    'Work',
    COALESCE((SELECT MAX(item_order) + 1 FROM spaces), 0),
    datetime('now')
WHERE NOT EXISTS (SELECT 1 FROM spaces WHERE id = '3');

PRAGMA foreign_keys = ON;
