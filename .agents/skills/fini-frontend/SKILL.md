---
name: fini-frontend
description: "Fini Vue frontend process for views, templates, rendering decisions, and tests. Use when creating or changing Vue frontend code under `src/`."
---

# Fini frontend

Use this skill when creating or changing Vue frontend code under `src/`,
especially view components, templates, conditional rendering, lists, and
frontend tests. It adds to `dev`; follow that process first.

The frontend rules are in `docs/code-style.md`, "Vue frontend":
DaisyUI and Tailwind first, `renderFlags`, `renderLists`, one named handler for
each event binding, no direct DOM queries, and component extraction.

1. Before you write template code, read `docs/code-style.md`, "Vue frontend".
2. Before you add a new template block of more than about ten lines, check
   "Component extraction" and extract the shared semantic controls first.
3. In the stabilize phase (`fini-test`, "Stabilize"), when the change added or
   changed render flags, cover the visible and hidden states as
   `docs/testing.md`, "Vue render states" says.
4. Before you hand off frontend template changes, check the diff against
   `docs/code-style.md`, "Review checklist", and run `npm run build` or the
   relevant frontend test target.
