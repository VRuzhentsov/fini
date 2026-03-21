# SpacesView

Legacy route: `/spaces`.

## Concept

Transitional spaces manager. Final MVP navigation keeps spaces management in [[SettingsView]] only.

Spaces are ordered by `item_order`. Built-in spaces use reserved ids (`1`, `2`, `3`) and cannot be deleted.

## Sections

### Space list
Each row shows the space name with Edit and Delete actions.
- **Edit** — inline: replaces the name with an input field. Confirm with Enter or Save button, cancel with Escape.
- **Delete** — hidden for built-ins (`id === "1"`, `"2"`, `"3"`). Removing custom space reassigns member quests to Personal (`space_id = "1"`).

### Add form
Text input + Add button at the bottom. Submits on Enter or button click. Clears after submit.

## Rules

| Rule | Detail |
|---|---|
| Built-ins | `id in {"1","2","3"}` — edit allowed, delete not |
| Empty name | Ignored on submit and on edit confirm |

## State

Uses [[space.ts]] store. Loads on mount.

## Dependencies

| Dep | Role |
|---|---|
| [[space.ts]] | Fetch, create, update, delete spaces |
