ALTER TABLE quests ADD COLUMN order_rank REAL NOT NULL DEFAULT 0;

WITH ordered AS (
    SELECT
        id,
        ROW_NUMBER() OVER (ORDER BY created_at ASC, id ASC) AS rn,
        COUNT(*) OVER () AS total
    FROM quests
)
UPDATE quests
SET order_rank = (
    SELECT
        CASE
            WHEN ordered.total <= 1 THEN 0.0
            ELSE ((CAST(ordered.rn AS REAL) - 1.0) / (CAST(ordered.total AS REAL) - 1.0)) * 200.0 - 100.0
        END
    FROM ordered
    WHERE ordered.id = quests.id
);
