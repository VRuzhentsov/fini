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

## Responsive placement

Behaves as a responsive side-sheet rather than a cursor-only popup (see the `Context menu — responsive side sheet` design card).

- **Trigger zone, not coordinates.** The trigger point (cursor for right-click, element centre for the three-dot button) classifies which zone of the app body it falls in (left/right × top/bottom). The menu snaps to that zone's side and vertical edge — the exact pixel position never pins it.
- **Width.** `min(50% app-window, 240px)`, floored at `160px`.
- **Vertical fit.** The menu may grow to fill the available body height (top inset → bottom inset). It scrolls internally only as a last resort; section heads stay pinned.
- **Reserved bottom inset.** `composer height + safe-area-inset-bottom + 8px`. Menus and submenus stop above it; they never overlap the fixed composer.
- **Submenu (wide).** Opens flush against the main menu in the opposite half of the app window, sharing the inner edge.
- **Narrow fallback.** When main + submenu cannot fit side-by-side at usable widths, the submenu opens as an in-place overlay over the main surface, pinned to the bottom inset and growing upward, with a back affordance labelled with the parent menu's title.
- **Mobile (≤ 640px).** The existing bottom action sheet (grip handle) still wins below the breakpoint; side-sheet rules apply above it.

## Dependencies

None — standalone. Consumers import `useContextMenu()` and provide their own items.
