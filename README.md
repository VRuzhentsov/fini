# Fini

**A quest-based productivity system for ADHD brains that helps people finish things.**

## The Problem

Traditional todo apps fail people with ADHD. They accumulate unfinished tasks, create guilt, and get abandoned within weeks. The more tasks pile up, the harder it becomes to open the app at all.

ADHD brains don't struggle with laziness — they struggle with task paralysis, energy management, and the inability to finish what they started.

## The Idea

Fini replaces the todo list with a quest system inspired by RPG games like Skyrim, Cyberpunk, and The Witcher. Instead of staring at a wall of obligations, you see one active quest at a time.

Quests are organized into **Spaces** — named contexts like Personal, Work, or any project. A Space is a lightweight container; quests belong to one space or none at all.

### Core Principles (Target — not all implemented yet)

- **One quest at a time.** No overwhelming lists. Just your current mission.
- **Spaces for context.** Group quests by area of life (personal, work, side project) without building a hierarchy.
- **Voice-first input.** Tap the mic, say what's on your mind. AI breaks it into small, achievable steps. _(planned)_
- **Energy-aware.** Tell the app how you feel today. Low energy = lighter quests. High energy = bigger chunks.
- **Abandon is okay.** Quests can be abandoned without guilt. Closing a chapter is a decision, not a failure. Completed and abandoned quests live in History — out of sight, but recoverable.
- **Zero guilt accumulation.** The app never shows you a pile of unfinished tasks. Ever.
- **The app leads, not you.** It tells you what to do next. No planning, no prioritizing, no organizing.
- **Privacy & cyber security.** Your brain is your business. Data is encrypted, the codebase is open for audit, and protection follows you across every device and platform.
- **Local-first.** Everything runs on your device. No accounts, no cloud sync required. Optional sync later, on your terms.

### What Makes Fini Different

| Traditional Todo App           | Fini                    |
| ------------------------------ | ----------------------- |
| Shows all tasks at once        | Shows one quest         |
| User organizes and prioritizes | AI handles structure    |
| Unfinished tasks pile up       | Quests can be abandoned |
| Text input                     | Voice-first             |
| Assumes constant energy        | Adapts to energy level  |
| Guilt-driven                   | Guilt-free              |

## Architecture

| Folder | Role |
|---|---|
| `src/` | Vue 3 frontend — see `src/README.md` |
| `src-tauri/` | Rust backend (Tauri 2.0) — see `src-tauri/README.md` |
| `spec/` | Domain model specs shared between frontend and backend |

Each folder has its own `README.md` with structure and conventions. Each significant source file has a companion `.md` spec — see **Spec files** below.

## Spec files

Every significant source file has a companion `.md` file with the same name (e.g. `App.vue` → [[App.md]]). These files are the **source of truth** for that file: they describe its purpose, the sections or structure it must contain, its props/events/commands, and any design decisions. Code should be written to match the spec, not the other way around.

Convention:
- **Domain model specs** live in `spec/` — shared between frontend and backend
- **UI specs** live next to the source file they describe (e.g. `App.vue` → `App.md`)
- A spec file for a view describes its concept and sections
- A spec file for a component describes its props, events, and behaviour
- A spec file for a store lists its actions
- Folder-level `README.md` files describe the folder's role and overall structure
- Use `[[wikilinks]]` liberally to cross-reference related specs — every mention of another file or concept should link to its spec

## Tech Stack

| Layer     | Technology                  |
| --------- | --------------------------- |
| Framework | Tauri 2.0                   |
| Frontend  | Vue 3 + TypeScript + Vite   |
| Styling   | Tailwind CSS + DaisyUI      |
| Icons     | Heroicons                   |
| State     | Pinia                       |
| Database  | SQLite via Diesel ORM       |
| Backend   | Rust                        |

## Target Platforms

- Linux (native + Flatpak)
- Windows
- Android
- macOS (planned)
- iOS (planned)

## Development

### Prerequisites

- Rust (via rustup)
- Node.js + npm
- Linux: webkit2gtk4.1-devel and related packages
- Android: Android Studio, JDK, NDK (see `src-tauri/gen/android/README.md`)

### Run (desktop)

```bash
npm ci
npm run tauri dev
```

### Build (desktop)

```bash
npm run tauri build
```

### Build (Android)

```bash
npm run tauri android build
```

### Build (Flatpak)

```bash
flatpak run org.flatpak.Builder --force-clean --user --install flatpak-build com.fini.app.yml
flatpak run com.fini.app
```

## Status

🚧 Early development. Building the MVP.

## Contributing

This is an open-source project. Contributions, ideas, and feedback are welcome.

If you have ADHD and want to help shape this product — you're exactly who we're building this for.

## License

TBD
