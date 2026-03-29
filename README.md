# Fini

**A quest-based productivity system for ADHD brains that helps people finish things.**

## The Problem

Traditional todo apps fail people with ADHD. They accumulate unfinished tasks, create guilt, and get abandoned within weeks. The more tasks pile up, the harder it becomes to open the app at all.

ADHD brains don't struggle with laziness — they struggle with task paralysis, energy management, and the inability to finish what they started.

```mermaid
flowchart TD
  A[Manual Set Focus: Q3] --> B[Reminder fires: Q2 becomes Focus]
  B --> C[Q3 becomes inactive before Q2 resolves]
  C --> D[Q2 gets completed]
  D --> E{Return target Q3 active?}
  E -- No --> F[Option 1: skip Q3 and fallback to next valid Focus]
  E -- No --> G[Option 2: clear Focus and wait for manual selection]
```
## The Idea

Fini replaces the todo list with a quest system inspired by RPG games like Skyrim, Cyberpunk, and The Witcher. Instead of staring at a wall of obligations, you see one active quest at a time.

Quests are organized into **Spaces** — named contexts like Personal, Work, or any project. A Space is a lightweight container; every quest belongs to exactly one space.

### Core Principles (Target — not all implemented yet)

- **One quest at a time.** No overwhelming lists. Just your current mission.
- **Spaces for context.** Group quests by area of life (personal, work, side project) without building a hierarchy.
- **Voice-first input.** Tap the mic, say what's on your mind. AI breaks it into small, achievable steps. _(post-MVP)_
- **Energy-aware.** Tell the app how you feel today. Low energy = lighter quests. High energy = bigger chunks.
- **Abandon is okay.** Quests can be abandoned without guilt. Closing a chapter is a decision, not a failure. Completed and abandoned quests live in History — out of sight, but recoverable.
- **Zero guilt accumulation.** The app never shows you a pile of unfinished tasks. Ever.
- **The app leads, not you.** It tells you what to do next. No planning, no prioritizing, no organizing.
- **Privacy & cyber security.** Your brain is your business. Fini is local-first with no cloud accounts. Encrypted LAN transport is part of sync work; local at-rest encryption is planned.
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

## Local Network Sync

Fini is local-first with optional LAN sharing. MVP.1 networking is split into:

- [[DeviceConnection]] for UDP discovery/pairing/presence
- [[SpaceSync]] for websocket-based per-space replication

See [[Network]] for transport-level contracts.

## MCP Server

Fini exposes a **Model Context Protocol (MCP) server** so external AI clients — primarily Claude Desktop — can read and manage quests directly.

### Entry point

`fini` — the single app binary. Its behaviour depends on how it is invoked:

| Invocation | Mode |
|---|---|
| `fini` | Launch GUI (Tauri) |
| `fini mcp` | Start MCP server over stdio, no GUI |

Claude Desktop launches `fini mcp` as a subprocess. Both modes share the same SQLite database at `$DATA_DIR/fini/fini.db`.

### Transport

`stdio` — launched as a subprocess by the MCP client via `fini mcp`.

### Tools

| Tool | Description |
|---|---|
| `list_quests` | Return actionable quests (including nearest open occurrence per series), optionally filtered by space |
| `get_quest` | Return a single quest by id |
| `create_quest` | Create a quest with title, due date, repeat rule, and optional explicit space (defaults to Personal `"1"`) |
| `update_quest` | Update any quest field (title, description, status, due, repeat, focus timestamps, etc.) |
| `delete_quest` | Delete a quest |
| `complete_quest` | Mark a quest completed |
| `abandon_quest` | Mark a quest abandoned |
| `list_history` | Return completed and abandoned quests |
| `get_active_focus` | Return the current Focus quest computed from focus history + fallback rules |
| `list_spaces` | Return all spaces |
| `create_space` | Create a space |
| `update_space` | Rename or reorder a space |
| `delete_space` | Delete a space |

### Tool outputs

MCP tools return structured JSON in `structured_content` (preferred) instead of human-formatted text.

`QuestRecord` fields: `id`, `series_id`, `occurrence_id`, `period_key`, `space_id`, `title`, `description`, `status`, `priority`, `energy`, `due`, `due_time`, `due_at_utc`, `repeat_rule`, `order_rank`, `completed_at`, `created_at`, `updated_at`.

`FocusHistoryRecord` fields: `id`, `device_id`, `quest_id`, `space_id`, `trigger`, `created_at`.

`SpaceRecord` fields: `id`, `name`, `item_order`, `created_at`.

Example `list_quests` output:

```json
{
  "quests": [
    {
      "id": "b0d3c9c6-1e6a-4bd2-9c77-5ef6b01b9e45",
      "series_id": "b0d3c9c6-1e6a-4bd2-9c77-5ef6b01b9e45",
      "occurrence_id": "b0d3c9c6-1e6a-4bd2-9c77-5ef6b01b9e45:2026-03-20",
      "period_key": "2026-03-20",
      "space_id": "1",
      "title": "Morning walk",
      "description": null,
      "status": "active",
      "priority": 1,
      "energy": "medium",
      "due": "2026-03-20",
      "due_time": "09:00",
      "due_at_utc": "2026-03-20T09:00:00Z",
      "repeat_rule": "daily",
      "order_rank": 0,
      "completed_at": null,
      "created_at": "2026-03-19T18:00:00Z",
      "updated_at": "2026-03-19T18:00:00Z"
    }
  ]
}
```

### Usage (Claude Desktop)

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "fini": {
      "command": "/path/to/fini",
      "args": []
    }
  }
}
```

The `fini` binary is produced by the normal Tauri build:

```bash
npm run tauri build
# binary at: src-tauri/target/release/fini
```

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
| MCP       | rmcp (Rust MCP SDK), stdio  |

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

# optional convenience flow
make android-sign-debug
make android-install-debug
make android-launch
```

Signed debug APK path from convenience flow: `bin/fini.apk`

### Build (Flatpak)

```bash
flatpak run org.flatpak.Builder --force-clean --user --install flatpak-build com.fini.app.yml
flatpak run com.fini.app
```

## Status

🚧 Early development. Building the MVP.

## Delivery Plan

- **MVP**: Local-first core loop on Linux/Windows/Android with functional parity (`Focus` / `History` / `Settings`, reminders, repeating series+occurrences, MCP for daily use)
- **MVP.1**: LAN pairing + sync (mutual confirmation, websocket data-plane, near-real-time replication, offline queue/replay, shared series occurrence behavior)
- Planning baseline and spec deltas are tracked in `docs/plans/2026-03-21-mvp-baseline.md`

## Contributing

This is an open-source project. Contributions, ideas, and feedback are welcome.

If you have ADHD and want to help shape this product — you're exactly who we're building this for.

## License

TBD
