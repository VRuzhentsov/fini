# src/

Vue 3 frontend of the Fini app. Built with TypeScript, Vite, Tailwind CSS, and DaisyUI.

## Structure

```
src/
├── main.ts                  # App entry point
├── App.vue                  # Root component — see [[App.md]]
├── router/
│   └── index.ts             # Route definitions
├── views/                   # Page-level route components
│   └── settings/            # Settings and the pages it opens
├── stores/                  # Pinia stores, one per domain
├── components/
│   ├── FocusView/           # Components specific to FocusView
│   │   ├── ActiveQuestPanel.vue
│   │   └── NewQuestForm.vue
│   ├── QuestsView/          # Transitional/shared list components
│   │   ├── QuestList.vue
│   │   └── RecurrenceScopeSheet.vue
│   ├── settings/            # Settings sections and dialogs
│   │   └── device/          # One paired device's page
│   └── ToastStack.vue       # Global toast notifications
└── composables/             # Shared Vue composables
```

View-specific components live in a subfolder named after their view. Shared components sit at the `components/` root. Where a group of views belongs to one area, the folder is named for the area rather than a single view and nests — `settings/`, with `settings/device/` under it; see `docs/naming.md`.

Current primary tabs are `Focus`, `History`, and `Settings`. Active backlog management is part of `Focus` (route remains `/main` during transition).

Each view, component, and store has a companion `.md` spec file.

## Conventions

- **Composition API** with `<script setup>` in all components
- **Pinia** for shared state — one store per domain (`quest`, `space`, etc.)
- **Tailwind CSS + DaisyUI** for styling — use DaisyUI component classes where available, Tailwind utilities for layout and custom elements
- **Heroicons** (`@heroicons/vue`) for icons — use the `24/outline` set, sized with `size-4` / `size-5`
- Communicate with the Rust backend via `invoke()` from `@tauri-apps/api/core`
- All store actions accept typed input objects matching the Rust command signatures

## Entry point

`main.ts` bootstraps the app:
1. Creates the Vue [[App]]
2. Registers Pinia and the router
3. Mounts to `#app` in `index.html`
