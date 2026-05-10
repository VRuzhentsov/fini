# ContextMenu

Global context menu rendered once in [[App.vue]]. Any component can open it via `useContextMenu()` composable with dynamic menu items.

## Layout

```
┌─────────────────┐
│ Item 1           │
│ Item 2         ▸ │
│ ┌──────────────┐ │
│ │ Sub-item A   │ │
│ │ Sub-item B   │ │
│ └──────────────┘ │
│ ─────────────── │
│ Item 3           │
└─────────────────┘
```

DaisyUI `menu` positioned absolutely at cursor. Supports flat items, submenus, separators.

## Menu item shape

| Field | Type | Description |
|---|---|---|
| `label` | `string` | Display text |
| `action` | `() => void` | Callback on click (leaf items) |
| `disabled` | `boolean?` | Greys out the item |
| `children` | `MenuItem[]?` | Submenu items (renders `▸` indicator) |
| `separator` | `boolean?` | Renders a divider instead of an item |

## Composable: `useContextMenu()`

Returns `{ open, close }`.

| Method | Signature | Description |
|---|---|---|
| `open` | `(event: MouseEvent, items: MenuItem[]) => void` | Opens menu at cursor with given items |
| `close` | `() => void` | Closes menu |

Internally writes to a shared reactive store read by the global `<ContextMenu>` instance.

## Behaviour

| Trigger | Effect |
|---|---|
| `open(event, items)` | Positions menu at `event.clientX/Y`, renders items, prevents default |
| Click item | Calls item `action`, closes menu |
| Click outside | Closes menu |
| Escape | Closes menu |
| Scroll | Closes menu |

On narrow screens, the bottom-sheet menu renders over a dimmed, blurred scrim so the menu reads as modal and the background recedes. Tapping the scrim closes the menu.

## Dependencies

None — standalone. Consumers import `useContextMenu()` and provide their own items.
