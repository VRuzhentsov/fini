# App.vue

Root component. Renders the tab bar and the current route view.

## Tabs

| Label | Route | View | Status |
|---|---|---|---|
| Main | `/main` | [[MainView]] | active |
| Quests | `/quests` | [[QuestsView]] | |
| History | `/history` | [[HistoryView]] | active |
| Spaces | `/spaces` | [[SpacesView]] | active |
| Settings | `/settings` | [[SettingsView]] | active |

## Notes

- Tab bar is always visible at the top
- `<router-view>` fills the remaining space below
- [[ToastStack]] is rendered globally outside the route view
