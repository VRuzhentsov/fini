# src/

Vue 3 frontend of the Fini app. Built with TypeScript, Vite, and Tailwind CSS.

## Structure

```
src/
├── main.ts                  # App entry point
├── App.vue                  # Root component — see [[App.md]]
├── router/
│   └── index.ts             # Route definitions
├── views/                   # Page-level components, one per tab
├── stores/                  # Pinia stores, one per domain
├── components/
│   ├── MainView/            # Components specific to MainView
│   │   ├── ActiveQuestPanel.vue
│   │   └── NewQuestForm.vue
│   ├── QuestsView/          # Components specific to QuestsView
│   │   └── QuestList.vue
│   ├── ChatInput.vue        # Shared input bar
│   └── ToastStack.vue       # Global toast notifications
└── composables/             # Shared Vue composables
```

View-specific components live in a subfolder named after their view. Shared components sit at the `components/` root.

Each view, component, and store has a companion `.md` spec file.

## Conventions

- **Composition API** with `<script setup>` in all components
- **Pinia** for shared state — one store per domain (`quest`, `space`, etc.)
- Communicate with the Rust backend via `invoke()` from `@tauri-apps/api/core`
- All store actions accept typed input objects matching the Rust command signatures

## Entry point

`main.ts` bootstraps the app:
1. Creates the Vue [[App]]
2. Registers Pinia and the router
3. Mounts to `#app` in `index.html`
