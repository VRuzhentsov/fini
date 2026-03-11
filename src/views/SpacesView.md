# SpacesView

Route: `/spaces`. Tab: Spaces. See [[App.md]].

## Concept

Manage named contexts that group quests. Spaces are ordered by `item_order`. The **default space** ("Personal", `id = 1`) is seeded at DB init and cannot be deleted.

## Sections

### Space list
Each row shows the space name with Edit and Delete actions.
- **Edit** — inline: replaces the name with an input field. Confirm with Enter or Save button, cancel with Escape.
- **Delete** — hidden for the default space (`id === 1`). Removes the space; quests in it become unassigned (`space_id = null`).

### Add form
Text input + Add button at the bottom. Submits on Enter or button click. Clears after submit.

## Rules

| Rule | Detail |
|---|---|
| Default space | `id === 1` ("Personal") — edit allowed, delete not |
| Empty name | Ignored on submit and on edit confirm |

## State

Uses [[space.ts]] store. Loads on mount.

## Dependencies

| Dep | Role |
|---|---|
| [[space.ts]] | Fetch, create, update, delete spaces |
