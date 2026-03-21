# App.vue

Root component. Renders the tab bar and the current route view.

## Tabs

| Label | Route | View | Status |
|---|---|---|---|
| Main | `/main` | [[MainView]] | active |
| History | `/history` | [[HistoryView]] | active |
| Settings | `/settings` | [[SettingsView]] | active |

## Notes

- Tab bar is always visible at the top
- `<router-view>` fills the remaining space below
- [[ToastStack]] is rendered globally outside the route view
- Active backlog browsing/editing lives in [[MainView]]
- Space management lives in the Spaces section of [[SettingsView]]
