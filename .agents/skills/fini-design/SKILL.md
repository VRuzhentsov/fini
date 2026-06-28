---
name: fini-design
description: |
  Canonical Fini design source = sibling `../fini-design/` HTML/CSS/JS handoff bundle exported from claude.ai/design.
  Load this skill on ANY of these triggers (auto-trigger first, ask second):
    1. User pastes a `https://api.anthropic.com/v1/design/h/<hash>` URL (a Claude Design export link).
    2. User asks to "fetch / read / sync / pull / refresh" a design or design bundle, with or without a URL.
    3. User asks to "implement / build / translate / port / recreate / wire up" a Fini surface — including
       Context menu, Quest card, Quest list, Composer / ChatInput, Top nav, Settings section, Device list,
       Device view, Reminder, Focus view, Active quest panel, Spaces, Energy, Voice model, History, or any
       prototype filename under `../fini-design/project/preview/`.
    4. User says "design a new surface", "refine [surface]", "audit visuals", or "clarify design intent" for Fini.
    5. User asks to update/close a Fini GitHub issue whose body cites a `../fini-design/` prototype or wiki
       grilling note (e.g. `fini-wiki/raw/*-grilling.md`).
  The bundle's own README enforces "read chats first, then primary file under `project/`, follow imports,
  do not screenshot, pixel-recreate not structurally copy" — this skill enforces the same protocol and chains
  to `sync-design-bundle` (in the bundle repo) when a fresh URL is supplied. Direct Figma editing via
  TalkToFigma is a secondary surface, used only when the user is actively in a Figma file or no bundle
  counterpart exists.
---

# Fini Design

Design source for Fini lives in the sibling `../fini-design/` repo as an HTML/CSS/JS handoff bundle exported from claude.ai/design. Treat that bundle as the source of truth. Figma is a secondary surface.

## Sibling Design Repo

Resolve the path the same way `fini-wiki` does:

```text
current repo: <repo-name>      (e.g. fini)
design repo:  ../<repo-name>-design/   (e.g. ~/projects/fini-design/)
```

If `../fini-design/` is missing, stop and tell the user — design work cannot proceed without it. Do not invent design intent.

### Bundle layout

| Path | Purpose |
|---|---|
| `README.md` | Authoritative handoff directives. Re-read on every session. |
| `chats/*.md` | Design intent transcripts. Read FIRST. |
| `project/README.md` | Design system reference: tone, vocabulary, principles, voice. |
| `project/preview/*.html` | Per-surface prototypes (`quest-card`, `reminder`, `composer`, `nav`, `settings-section`, `device-list`, `device-view`, etc.). |
| `project/preview/_*.{css,html,js}` | Shared partials imported by prototype pages. |
| `project/colors_and_type.css` | Token definitions (color, type, scale). |
| `project/ui_kits/app/` | Assembled kit (`Fini App.html`, `index.html`, `kit.css`, `Components.jsx`, `_theme.js`). |
| `project/assets/` | Icons, app icon. |
| `project/uploads/` | Supplemental references (pasted images, grilling notes). |
| `project/tmp/` | Scratch screenshots. Ignore unless the chat cites them. |

## Required intake order

Enforce this every time the bundle is consulted. The bundle's own README mandates it.

1. Read every file in `chats/` end-to-end. The chat is where intent lives; the prototypes are the output.
2. Read `project/README.md` for vocabulary, voice, and product principles.
3. Identify the primary file from chat — the surface the user was last iterating on.
4. Read the primary HTML top to bottom. Follow every `<link>`, `<script>`, and partial import (anything starting with `_`).
5. Read `project/colors_and_type.css` for token semantics that any prototype consumes.
6. Do not render prototypes in a browser and do not take screenshots unless the user explicitly asks. Read source directly — it spells out dimensions, colors, layout rules.

If the bundle changes between sessions, re-run intake; do not rely on memory of an earlier import.

## Implementation guidance

The prototypes are reference, not production code. Recreate the visual output in the target codebase's actual stack — for the Fini app that means Vue 3 + Tailwind 4 + DaisyUI 5 + Heroicons.

- Match visual output pixel-for-pixel. Do not copy the prototype's HTML structure unless it happens to fit the target.
- Map prototype tokens to the existing app tokens. Do not introduce a parallel token system. If a prototype token has no app equivalent, surface that as a question before adding one.
- Reuse existing Vue components in `src/components/` first. Read before creating.
- In Vue templates, start with DaisyUI component classes for known controls/surfaces and Tailwind utilities for layout, spacing, sizing, states, and small visual adjustments.
- Use scoped custom CSS/classes only when the framework utilities cannot achieve the needed quality, when an existing semantic component class should be reused, or when a narrow interaction/layout behavior would be less maintainable as utilities.
- Keep custom CSS small and local. Avoid broad one-off class taxonomies, parallel style systems, and copying prototype class names into production Vue.
- Match the exact vocabulary in `project/README.md` ("Quest", "Space", "Focus", "Abandon", etc.). Do not rename product concepts.
- Sentence case for buttons and labels; uppercase tracked-out only for app-chrome section headers (`SPACES`, `DEVICES`, `VOICE MODEL`).
- When validating screen placement, sticky/fixed positioning, keyboard/safe-area behavior, or whether a component belongs inside a list vs app chrome, use a full viewport screenshot with surrounding UI. A cropped component screenshot can supplement details, but it is not valid placement evidence by itself.

## Ambiguity protocol

If the chat does not pin intent, the prototype is incomplete, or two prototypes contradict, ask the user one targeted question via the `question` tool before implementing. Cheaper to clarify than to build the wrong thing — the bundle README says so explicitly.

Common ambiguity sources:
- Multiple prototypes for the same surface (which is canonical).
- Tokens defined in `colors_and_type.css` but not used in any prototype.
- Chat ends mid-iteration with no clear "we landed here".

## Figma fallback (TalkToFigma)

Use TalkToFigma only when one of these is true:
- The user is actively editing a specific Figma file and asks for help in it.
- The bundle has no counterpart for the surface and the user wants to mock first in Figma.
- The user wants component-set cleanup, variant property work, or visual QA inside Figma.

When using TalkToFigma:
1. `TalkToFigma_join_channel` to confirm connection. Report disconnect state if blocked.
2. Inspect first — read component sets, frames, instances, properties before editing.
3. Make the smallest structural change that fixes the problem. Prefer fixing the shared definition over per-variant patching.
4. For actions TalkToFigma cannot perform (variant property panels, browser-only UI), describe the exact manual step for the user, wait for confirmation, then re-read nodes to verify.
5. Verify by re-reading affected nodes. Never claim a fix from intent alone.

The Figma file follows the bundle, not the other way around. If Figma and the bundle disagree, the bundle wins unless the user explicitly says Figma is now ahead.

## Boundaries

This skill owns:
- Reading the bundle and translating intent into an implementation plan.
- Deciding token and component mapping between bundle and app.
- Light Figma edits via TalkToFigma when the user is in a Figma file.

Hand off when:
- Multi-file Vue implementation work begins → pair with `frontend-design` for code generation patterns.
- Direct Figma-to-code generation against a Figma node → `figma:figma-implement-design`.
- Production code review of the resulting Vue → standard repo review flow via `fini-dev`.

Per `fini-dev` routing, design-to-code work loads this skill first.
