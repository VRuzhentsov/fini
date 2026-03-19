# NewQuestForm

Thin wrapper that connects [[ChatInput]] to quest creation. Used in [[MainView]].

## Layout

```
[ Space selector ▾ ]
[ Chat input field  ] [Send]
```

Space selector sits directly above the input row.

## Space selector

- DaisyUI `select select-sm` dropdown listing all spaces from `spaces` store
- Default: "Personal" space, or the last space the user selected (persisted in `localStorage` as `lastSpaceId`)
- On mount: restore `lastSpaceId` from `localStorage`; fall back to the space named "Personal"; fall back to first space
- On change: save selection to `localStorage`

## Behaviour

On `submit` from [[ChatInput]], calls `createQuest({ title: text, space_id: selectedSpaceId })` via [[quest.ts]].

## Dependencies

| Dep           | Role                     |
| ------------- | ------------------------ |
| [[ChatInput]] | Input UI, emits `submit` |
| [[quest.ts]]  | `createQuest`            |
| [[spaces.ts]] | Space list for selector  |
