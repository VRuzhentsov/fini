# src/

Vue 3 frontend of the Fini app. Built with TypeScript, Vite, and Tailwind CSS.

## Structure

```
src/
├── main.ts        # App entry point — mounts Vue app, registers Pinia
├── App.vue        # Root component
├── style.css      # Global styles (Tailwind base import)
├── assets/        # Static assets (icons, images)
└── vite-env.d.ts  # Vite TypeScript declarations
```

## Planned structure (as the app grows)

```
src/
├── components/    # Reusable UI components
├── views/         # Page-level components (one per route)
├── stores/        # Pinia stores (state management)
├── composables/   # Shared Vue composables
└── lib/           # Utility functions and Tauri command wrappers
```

## Key conventions

- **Composition API** with `<script setup>` in all components
- **Tailwind CSS** for all styling — avoid inline styles and scoped CSS unless necessary
- **Pinia** for shared state — one store per domain (quest, energy, settings, etc.)
- Communicate with the Rust backend via `invoke()` from `@tauri-apps/api/core`

## Entry point

`main.ts` bootstraps the app:
1. Creates the Vue app
2. Registers Pinia
3. Imports global styles
4. Mounts to `#app` in `index.html`
