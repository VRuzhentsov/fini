# NewQuestForm

Thin wrapper that connects [[ChatInput]] to quest creation. Used in [[FocusView]].

## Layout

```
[ Space selector ▾ ]
[ Chat input field  ] [Send]
```

Space selector sits directly above the input row.

## Space selector

- DaisyUI `select select-sm` dropdown listing all spaces from `spaces` store
- Default: built-in Personal space (`id = "1"`), or the last selected space (`localStorage.lastSpaceId`)
- On mount: restore `lastSpaceId`; fall back to built-in Personal; fall back to first available space
- On change: save selection to `localStorage`

## Behaviour

On `submit` from [[ChatInput]], calls `createQuest({ title: text, space_id: selectedSpaceId })` via [[quest.ts]].

## Dependencies

| Dep           | Role                     |
| ------------- | ------------------------ |
| [[ChatInput]] | Input UI, emits `submit` |
| [[quest.ts]]  | `createQuest`            |
| [[spaces.ts]] | Space list for selector  |
