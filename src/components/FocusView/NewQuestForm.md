# NewQuestForm

Rich draft composer for Quest creation. Used in [[FocusView]] as the persistent bottom quick-capture bar.

## Layout

Collapsed quick-create state:

```
[ title input                                  ] [ Space ▾ ]
[ Date ] [ More ]                                        [ Send ]
```

Expanded metadata state:

```
[ title input                                  ] [ Space ▾ ]
[ description textarea                                      ]
[ shortcut hint                                             ]
[ Date / reminder ] [ Less ]                            [ Send ]
```

The composer uses the same compact card language as [[QuestEditor]] instead of the bottom chat-only input. Metadata expands in place so the draft still feels like one Quest being completed, not a separate form.

## Space selector

- Controlled [[SpacePicker]] listing concrete spaces from `spaces` store.
- Default: the current global selected space when present; otherwise built-in Personal (`id = "1"`); otherwise the first loaded space.
- Selection is local to the draft and does not mutate the global Space filter.
- Empty drafts follow changes to the global Space filter so quick-capture creates into the visible filtered Space.
- If a filter change happens while the draft has content, the draft resyncs to the current filter as soon as it becomes empty again.

## Reminder / date

- `Date` opens [[ReminderMenu]] against a draft Quest object.
- Saving the reminder stores `due`, `due_time`, and `repeat_rule` in draft state.
- Clearing the reminder resets those draft fields before creation.

## Behaviour

On submit, calls `createQuest({ title, description, space_id, due, due_time, repeat_rule })` via [[quest.ts]].

Description is optional. Whitespace-only descriptions are saved as `null`.

Empty titles cannot create a Quest.
While a create request is pending, the composer disables submit controls and ignores duplicate submits.

After a successful create:

- title is cleared
- description is cleared
- reminder fields are cleared
- composer returns to collapsed quick-create state
- selected Space resyncs to the current global Space filter for the next empty draft

## Dependencies

| Dep              | Role                                    |
| ---------------- | --------------------------------------- |
| [[quest.ts]]     | `createQuest` and draft Quest typing    |
| [[spaces.ts]]    | Space list and default draft space      |
| [[SpacePicker]]  | Local draft Space dropdown              |
| [[ReminderMenu]] | Date/time/repeat picker for draft state |
