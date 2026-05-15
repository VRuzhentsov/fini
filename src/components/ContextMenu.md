# ContextMenu

Global context menu rendered once in [[App.vue]]. Any component can open it via `useContextMenu()` composable with dynamic menu items.

## Layout

One adaptive surface — renders as a **side-sheet** (> 640px app width) or **mobile bottom-sheet** (≤ 640px). Submenus are an in-place accordion at every width. No flyout, no separate overlay.

```
┌────────────────────┐
│ ✓  Complete        │
│ ⚡  Set Focus      │
│ ↔  Move to space › │  ← accordion; click expands children below
│    ┌──── Personal  │
│    ┌──── Family ✓  │  ← checkmark on current pick
│    ┌──── Work      │
│ ─────────────────  │
│ ×  Abandon         │
│ ─────────────────  │
│ 🗑  Delete          │  ← danger row: quiet at rest, red wash on hover
└────────────────────┘
```

## Menu item shape

| Field | Type | Description |
|---|---|---|
| `label` | `string` | Display text (sentence case) |
| `action` | `() => void` | Callback on click (leaf items) |
| `disabled` | `boolean?` | Greys out the item |
| `children` | `MenuItem[]?` | Accordion children (chevron appears) |
| `separator` | `boolean?` | Renders a divider |
| `icon` | `Component?` | Heroicon (outline 24, rendered 16px) |
| `value` | `string?` | Trailing dim text — current pick of a picker row |
| `badge` | `string \| number?` | Trailing outlined pill |
| `selected` | `boolean?` | Trailing checkmark (for accordion children) |
| `loading` | `boolean?` | Trailing spinner (async rows) |
| `danger` | `boolean?` | Destructive row — normal at rest, red wash on hover |
| `spaceColor` | `string?` | Small color dot for space-picker children (`var(--space-color-*)`) |

## Composable: `useContextMenu()`

Returns `{ open, openFromRect, close }`.

| Method | Signature | Description |
|---|---|---|
| `open` | `(event: Event, items: MenuItem[]) => void` | Opens at cursor / element-center |
| `openFromRect` | `(rect: DOMRect, items: MenuItem[]) => void` | Opens from an explicit rect |
| `close` | `() => void` | Closes menu |

## Behaviour

| Trigger | Effect |
|---|---|
| `open(event, items)` | Classifies zone, renders surface, prevents default |
| Click leaf item | Calls `action`, closes menu |
| Click parent item | Toggles accordion (single-open) |
| Click accordion child | Collapses accordion, calls `action`, closes menu |
| Click scrim / outside | Closes menu |
| Escape | Closes menu |
| Scroll | Closes menu |
| Drag handle (mobile) ↓ > 120px or velocity > 0.6 | Swipe-to-dismiss |

## Responsive placement

- **Cursor-anchored.** Menu opens at the trigger point; only shifts when it would overflow.
- **Pointer trigger (right-click).** Top-left of menu at the cursor. If overflow: shift left/up to fit within body bounds.
- **Element trigger (button click).** Menu drops below the rect. Horizontal: left-align if room to the right of the button; right-align otherwise. Vertical: flip above the button when no room below (≥ 80px above required); otherwise pin to top with internal scroll.
- **Width.** `min(50% app-window, 240px)`, floored at `160px`.
- **Reserved bottom inset.** `composer height + safe-area-inset-bottom + 8px`. Menu never overlaps composer.
- **Submenu (all widths).** In-place accordion; single-open. Parent stays highlighted while open. Children indented, checkmark on `selected`.
- **Mobile (≤ 640px).** Bottom-sheet form — rounded top, drag handle, drag-to-dismiss (release > 120px or pointer velocity > 0.6 px/ms). Sheet grows to fit up to 85% body height, then scrolls internally. Cursor position ignored — always full-width at viewport bottom.

## Motion

| Transition | Duration | Notes |
|---|---|---|
| Side-sheet open | 160ms ease | `scale(0.96→1) + opacity 0→1` |
| Bottom-sheet entrance | 260ms `cubic-bezier(.34,1.4,.64,1)` | Spring-ish, gentle overshoot |
| Accordion expand/collapse | 200ms ease | `grid-template-rows: 0fr ↔ 1fr` |
| Chevron rotate | 200ms ease | `rotate(0 → 90deg)` paired with accordion |
| Drag | 1:1 to pointer | `translateY` tracks pointer; ≤30px rubber-band above rest |
| `prefers-reduced-motion` | 0ms | All of the above collapse to instant show/hide |

## Dependencies

None — standalone. Consumers import `useContextMenu()` and provide their own items.
