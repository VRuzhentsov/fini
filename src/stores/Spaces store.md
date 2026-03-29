Frontend state for [[Space]]. Single source of truth for space data in the UI.

## Actions

| Action | Description |
|---|---|
| `fetchSpaces()` | Load all spaces from the backend |
| `createSpace(name)` | Create a new space; appends it to `spaces` |
| `updateSpace(id, patch)` | Update a space; replaces it in `spaces` |
| `deleteSpace(id)` | Delete a space; removes it from `spaces` |

## Computed

| Getter | Description |
|---|---|
| `selectedSpaceId` | Active space filter from [[SpacePicker]], `null` = all spaces |

## Notes

- For the domain model see [[Space]].
- Space ids are string-based (`"1"`, `"2"`, `"3"`, or UUID).
- `selectedSpaceId` resets to `null` (All spaces) on each app restart.
