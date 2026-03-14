# Space

An area — a broad slice of the user's life or work that quests belong to. Examples: "Personal", "Work", "Project #4". Areas are user-defined and open-ended; they are not a strict project or category system.

Quests are optionally assigned to a space. The default space is "Personal" and is always present.

## Fields

| Field | Type | Description |
|---|---|---|
| `id` | integer | Unique identifier |
| `name` | string | Display name |
| `item_order` | integer | Sort position among spaces |
| `created_at` | datetime | |

## Rules

| Rule | Detail |
|---|---|
| Default space | `id = 1` ("Personal") — always present, cannot be deleted |
| Unassigned quests | `space_id = null` — valid; quests do not require a space |
| Deleted space | Quests in the deleted space become unassigned (`space_id = null`) |
