# Space

An area — a broad slice of the user's life or work that quests belong to. Examples: "Personal", "Work", "Project #4". Areas are user-defined and open-ended; they are not a strict project or category system.

Quests are optionally assigned to a space. Some spaces are built-in and non-deletable; they exist on every Fini install under the same name and serve as the common ground for [[Network]] sync.

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
| Built-in spaces | "Personal" and "Family" — always present, cannot be deleted, exist on every install |
| Unassigned quests | `space_id = null` — valid; quests do not require a space |
| Deleted space | Quests in the deleted space become unassigned (`space_id = null`) |
