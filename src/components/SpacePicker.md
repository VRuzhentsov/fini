# SpacePicker

Shared space dropdown rendered in the top bar and in local draft controls.

Without props it controls the global active space filter. With `v-model`, it becomes a controlled dropdown for local space selection without mutating the global filter.

## Layout

```
[ All spaces ▾ ] or [ Space ▾ ]
```

DaisyUI `dropdown` with the same chip/menu treatment in each consumer. Global mode lists "All spaces" + every space from [[spaces.ts]]. Local mode can set `allowAll = false` to list only concrete spaces.

## State

- `selectedSpaceId`: `string | null` — `null` means "All spaces"
- Resets to `null` on each app restart (not persisted)
- `modelValue`: optional controlled value for local consumers
- `allowAll`: whether the `null` / "All spaces" option is available

## Behaviour

| Action | Effect |
|---|---|
| Select a space | Sets `selectedSpaceId`, persists to localStorage |
| Select "All spaces" | Sets `selectedSpaceId = null` |
| Select a space in controlled mode | Emits `update:modelValue` only |

## Consumers

| Consumer | How it uses `selectedSpaceId` |
|---|---|
| [[FocusView]] | Filters active quest + backlog |
| [[QuestsView]] | Filters quest list |
| [[NewQuestForm]] | Uses controlled mode for the draft `space_id` and follows the global filter only while the draft is empty |

## Dependencies

| Dep | Role |
|---|---|
| [[spaces.ts]] | Space list |
