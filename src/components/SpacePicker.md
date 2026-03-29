# SpacePicker

Global space selector rendered in the top bar (right-aligned). Controls the active space filter and default space for new quests.

## Layout

```
[ All spaces ▾ ]
```

DaisyUI `dropdown` with a `menu` listing "All spaces" + every space from [[spaces.ts]].

## State

- `selectedSpaceId`: `string | null` — `null` means "All spaces"
- Resets to `null` on each app restart (not persisted)

## Behaviour

| Action | Effect |
|---|---|
| Select a space | Sets `selectedSpaceId`, persists to localStorage |
| Select "All spaces" | Sets `selectedSpaceId = null` |

## Consumers

| Consumer | How it uses `selectedSpaceId` |
|---|---|
| [[FocusView]] | Filters active quest + backlog |
| [[QuestsView]] | Filters quest list |
| [[NewQuestForm]] | Uses as default `space_id` for new quests (falls back to `"1"` when null) |

## Dependencies

| Dep | Role |
|---|---|
| [[spaces.ts]] | Space list |
