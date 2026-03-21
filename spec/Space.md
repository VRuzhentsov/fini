# Space

Named context that quests belong to. Spaces are shareable units for LAN sync selection.

## Fields

| Field | Type | Description |
|---|---|---|
| `id` | string | Reserved built-in id (`"1"`, `"2"`, `"3"`) or UUID for custom spaces |
| `name` | string | Display name (renamable, including built-ins) |
| `item_order` | integer | Sort position |
| `created_at` | datetime | |

## Built-in spaces

| Id | Default name | Deletable | Renamable |
|---|---|---|---|
| `"1"` | Personal | No | Yes |
| `"2"` | Family | No | Yes |
| `"3"` | Work | No | Yes |

## Rules

| Rule | Detail |
|---|---|
| Default assignment | New quests default to space `"1"` unless user picks another space |
| Unassigned quests | Not allowed; quest `space_id` is never null |
| Deleted custom space | Quests in deleted custom space are reassigned to built-in Personal (`space_id = "1"`) |
| Built-in rename sync | Built-in space renames replicate across paired devices |
