---
name: fini-documentation
description: "How Fini writes documentation: where each kind of doc lives (docs/), the STE writing rules, one place per topic, mermaid for diagrams, and approval for new doc files. Use before adding, moving, or changing any doc, spec, README, ADR, glossary entry, or architecture page in the Fini repo."
---

# Fini Documentation

Use this skill before you add, move or change documentation in this repo.

## Where documents go

All documents are in `docs/`:

| Kind | Place |
|---|---|
| Architecture: how a part of the app works | `docs/architecture/<part>.md` (for example `communication.md`) |
| Decision records | `docs/adr/` — written only together with the project owner, while a plan is discussed |
| What a word means | `docs/glossary.md` |
| How names are written | `docs/naming.md` |
| The writing standard | `docs/ste.md` |
| Plans of past work | `docs/plans/` |

A README or a companion spec next to the code is an older form. When you
change one, move its content to the right place in `docs/`, delete it from the
code tree, and update the links to it. Do not start new docs next to code.

## How to write

- Write in STE (ASD-STE100): read `docs/ste.md` first. Use short sentences,
  one instruction in each sentence, the active voice, and approved words. A
  technical noun or verb that is not in the STE dictionary must be in
  `docs/glossary.md`.
- One place for each topic. Other documents link to it; they do not repeat it.
- Write what the code does now. Put what is not done yet in a short "Not done
  yet" section, not between the facts.
- Name code by its real names (types, functions, files), so that a reader can
  find it.
- Use mermaid for diagrams. Keep to the number of new diagrams that the plan
  or the person gives.

## Approval

- A new document file needs the approval of the project owner first: give
  its name and its job.
- A new term in the glossary needs the approval of the project owner first.
- Changes to an ADR happen only while you discuss a plan with the project
  owner.

## Check

- Every link in the changed documents goes to a file that exists.
- No document repeats a topic that has its own place.
