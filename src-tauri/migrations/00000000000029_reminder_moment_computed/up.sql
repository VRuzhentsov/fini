-- A reminder's moment is computed when it is read (docs/db.md; the form of
-- iCalendar VALARM and of Todoist): a relative reminder fires at its quest's
-- due + due_time minus its offset; only an absolute reminder keeps a moment
-- of its own.
--
-- The reminder that a quest's date made was stored as an 'absolute' row,
-- with no offset and a copy of the quest's moment: the first reminder of a
-- quest with a due date (the old code updated the quest's first row). It
-- becomes a relative reminder at the due time (offset 0). Any other absolute
-- reminder was made with its own time and keeps it. A relative reminder
-- keeps no moment.
UPDATE reminders
SET type = 'relative', mm_offset = 0, due_at_utc = NULL
WHERE type = 'absolute' AND mm_offset IS NULL
  AND quest_id IN (SELECT id FROM quests WHERE due IS NOT NULL)
  AND rowid IN (SELECT MIN(rowid) FROM reminders GROUP BY quest_id);

UPDATE reminders
SET due_at_utc = NULL
WHERE type = 'relative';
