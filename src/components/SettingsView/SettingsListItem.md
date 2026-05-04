# SettingsListItem

Primitive row for Settings screens.

## Purpose

Keep Settings rows to one of two content shapes:

- One-column default slot
- Two-column `start` / `end` key-value layout

Fixed-size row chrome can be supplied through `leading` and `trailing` slots.

## Props

| Prop | Type | Meaning |
|---|---|---|
| `to` | `string` | Render the whole row as a `RouterLink` |
| `href` | `string` | Render the whole row as an external link |
| `button` | `boolean` | Render the whole row as a button and emit `click` |

Rows with nested inputs, menus, or buttons should stay passive and put controls inside slots.

## Layout Rules

- `start` is flexible and owns remaining width
- `end` is right-aligned and capped at `50%`
- Rows stay horizontal on narrow screens
- Controls and indicators belong in fixed `leading` / `trailing` chrome or inside `start` / `end`

## Theme

Uses DaisyUI/Tailwind theme tokens directly. It does not receive theme props and does not compare light/dark/system in the template.
