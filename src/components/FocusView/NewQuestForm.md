# NewQuestForm

Rich draft composer for Quest creation. Used in [[FocusView]].

## Layout

```
[ checkbox affordance ] [ title input                    ] [ Space ▾ ]
[ Date / reminder ]                                      [ Send ]
```

The composer uses the same compact card language as [[QuestEditor]] instead of the bottom chat-only input.

## Space selector

- Inline `select` listing all spaces from `spaces` store.
- Default: the current global selected space when present; otherwise built-in Personal (`id = "1"`); otherwise the first loaded space.
- Selection is local to the draft and does not mutate the global Space filter.

## Reminder / date

- `Date` opens [[ReminderMenu]] against a draft Quest object.
- Saving the reminder stores `due`, `due_time`, and `repeat_rule` in draft state.
- Clearing the reminder resets those draft fields before creation.

## Behaviour

On submit, calls `createQuest({ title, space_id, due, due_time, repeat_rule })` via [[quest.ts]].

Empty titles cannot create a Quest.

After a successful create:

- title is cleared
- reminder fields are cleared
- selected Space is preserved for the next draft

## Dependencies

| Dep              | Role                                    |
| ---------------- | --------------------------------------- |
| [[quest.ts]]     | `createQuest` and draft Quest typing    |
| [[spaces.ts]]    | Space list and color classes            |
| [[ReminderMenu]] | Date/time/repeat picker for draft state |
