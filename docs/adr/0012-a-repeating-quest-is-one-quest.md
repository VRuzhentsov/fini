# 0012 — A repeating quest is one quest

## Status

Accepted. Written during the design discussion of 2026-10-30. Answers issue
#150.

## Context

A repeating quest is two kinds of row today: one `quest_series` row (the
rule and a template) and one `quests` row for each occurrence. When a person
completes an occurrence, `generate_next_occurrence` writes the row of the
next one. After missed days the person must complete each missed occurrence
one at a time before the quest reaches today.

The person wants a repeating quest to work as a repeating event in Google
Calendar or Outlook: one event with a rule, and occurrences that the
calendar computes from the rule. Nothing must appear in the database only
because a quest exists. The database keeps what a person did.

Designs compared: Google Calendar API (one event with `recurrence`; a changed
instance is an exception event with `recurringEventId` and
`originalStartTime`), iCalendar RFC 5545 (RRULE, and an override VEVENT with
RECURRENCE-ID), Outlook (a series and its exceptions), Obsidian Daily Notes
(a note for each day, made from a template).

## Decisions

**D1 — One entity.** A repeating quest is one `quests` row with its
`repeat_rule`, as a one-time quest is. `quest_series` and
`series_reminder_templates` are removed, and `quests.series_id` and
`quests.period_key` too. An edit of the quest changes all its occurrences,
past and future.

**D2 — Occurrences are computed.** The open occurrences of a quest are
computed from its rule and its stored occurrence marks, as a Vue `computed`
value is computed from its sources. An occurrence is a date of the quest
(`period_key`, `YYYY-MM-DD`).

**D3 — An occurrence mark is stored only after a person acts.** The table
`quest_occurrences (quest_id, period_key)` holds the occurrences that a
person closed (completed or abandoned) or changed (a tick, a "This day"
edit). Complete has no other effect: it writes no next occurrence.

**D4 — Sparse overrides.** A mark has the fields of a quest that can change
for one day (title, description, energy, priority, due time). An empty
field takes the value of the quest. Unlike a Google exception, a mark is not
a full copy, so a later edit of the quest reaches the fields that the day
did not change.

**D5 — The description is a markdown file.** The checklist lives only in the
description. The first tick on a day writes that day's own copy of the
description, with its ticks, like a daily note made from a template. An
edit of the quest's description reaches a day copy through the 3-way merge
(`checklist_base` and `merge_3way`).

**D6 — Focus and the lists show the oldest open occurrence.** Set Focus
points at the quest and writes no mark.

**D7 — Complete through today.** A new action closes every open occurrence
dated today or before, today's included, whatever its time. Each one gets
its own completed mark. The action is in the menu of the Complete button on
Focus (hold) and in the context menu of a repeating quest. A tap on Complete
closes one occurrence.

**D8 — Edit scope.** An edit of a repeating quest asks "This day" (a mark
with an override) or "All days" (the quest). A tick does not ask.

**D9 — Reminders belong to the quest.** A reminder keeps its rule (kind,
offset). The scheduler computes the next fire time in the future among the
open occurrences and keeps one notification for each reminder.

**D10 — Stop and change.** To stop the repeat gives the rule an end at the
last closed occurrence; its marks stay. A quest with no marks becomes a
one-time quest when its repeat is removed. A changed rule computes the open
occurrences from the new rule, from today; the old marks stay.

**D11 — Sync and the old data.** A mark syncs as the entity
`quest_occurrence`, keyed by (quest_id, period_key), through today's outbox
(issue #196 replaces the outbox later). The sync protocol version goes to
`fini/sync/5`. The migration converts the old data: each series becomes one
quest with the id of the series; each closed occurrence becomes a mark; an
open occurrence becomes a mark only when it has ticks; focus history and
reminders move to the quest.

## Consequences

- A quest that nobody touches writes nothing. A week of missed days is one
  action to close.
- History of an old day shows the quest's present title, unless that day
  has its own.
- Every reader of occurrences (Focus, lists, History, reminders, sync,
  backup, CLI) reads (quest, date) instead of an occurrence row.
- An old and a new device do not sync until both have the new version.
