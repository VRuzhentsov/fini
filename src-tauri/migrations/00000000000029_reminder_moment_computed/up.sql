-- A reminder's moment is computed when it is read (docs/db.md; the form of
-- iCalendar VALARM and of Todoist): a relative reminder fires at its quest's
-- due + due_time minus its offset; only an absolute reminder keeps a moment
-- of its own.
--
-- The reminders that a quest's date made were stored as 'absolute' rows,
-- with no offset and a copy of the quest's moment. They become relative
-- reminders at the due time (offset 0). A relative reminder keeps no moment.
UPDATE reminders
SET type = 'relative', mm_offset = 0, due_at_utc = NULL
WHERE type = 'absolute' AND mm_offset IS NULL;

UPDATE reminders
SET due_at_utc = NULL
WHERE type = 'relative';
