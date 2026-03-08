# Fini

**A quest-based productivity system for ADHD brains that helps people finish things.**

## The Problem

Traditional todo apps fail people with ADHD. They accumulate unfinished tasks, create guilt, and get abandoned within weeks. The more tasks pile up, the harder it becomes to open the app at all.

ADHD brains don't struggle with laziness — they struggle with task paralysis, energy management, and the inability to finish what they started.

## The Idea

Fini replaces the todo list with a quest system inspired by RPG games like Skyrim, Cyberpunk, and The Witcher. Instead of staring at a wall of obligations, you see one active quest with clear steps.

### Core Principles (Target — not all implemented yet)

- **One quest at a time.** No overwhelming lists. Just your current mission and the next step.
- **Voice-first input.** Tap the mic, say what's on your mind. AI breaks it into small, achievable steps.
- **Energy-aware.** Tell the app how you feel today. Low energy = tiny steps. High energy = bigger chunks.
- **Abandon is okay.** Quests can be paused or released without guilt. Closing a chapter is a decision, not a failure.
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

```
fini/
├── src/                   # Vue 3 frontend (TypeScript, Tailwind CSS)
├── src-tauri/             # Rust backend (Tauri 2.0)
│   ├── src/               # Rust source code
│   ├── gen/android/       # Generated Android project
│   └── icons/             # App icons for all platforms
├── com.fini.app.yml       # Flatpak manifest
└── com.fini.app.desktop   # Linux desktop entry
```

See each folder's `README.md` for details.

## Tech Stack

| Layer     | Technology                  |
| --------- | --------------------------- |
| Framework | Tauri 2.0                   |
| Frontend  | Vue 3 + TypeScript + Vite   |
| Styling   | Tailwind CSS                |
| State     | Pinia                       |
| Database  | SQLite via tauri-plugin-sql |
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
