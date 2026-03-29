# App.vue

Root component. Renders the tab bar and the current route view.

## Tabs

| Label | Route | View | Status |
|---|---|---|---|
| Focus | `/main` | [[MainView]] | active |
| History | `/history` | [[HistoryView]] | active |
| Settings | `/settings` | [[SettingsView]] | active |

## Notes

- Tab bar is always visible at the top, with [[SpacePicker]] right-aligned
- `<router-view>` fills the remaining space below
- [[ContextMenu]] is rendered globally once, opened by any component via `useContextMenu()`
- [[ToastStack]] is rendered globally outside the route view
- Focus quest + active backlog browsing/editing lives in [[MainView]]
- Space management lives in the Spaces section of [[SettingsView]]
