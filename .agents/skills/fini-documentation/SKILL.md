---
name: fini-documentation
description: "How Fini writes documentation: where each kind of doc lives (docs/), the STE writing rules, one place per topic, ASCII trees for architecture (mermaid only for complex algorithms), and approval for new doc files. Use before adding, moving, or changing any doc, spec, README, ADR, glossary entry, or architecture page in the Fini repo."
---

# Fini Documentation

Use this skill before you add, move or change documentation in this repo.

## Where documents go

All documents are in `docs/`:

| Kind | Place |
|---|---|
| Architecture: how a part of the app works | `docs/architecture/<part>.md` (for example `communication.md`) |
| How code is shaped, in all parts of the app | `docs/architecture/code-design.md` |
| Decision records | `docs/adr/` — written only together with the project owner, while a plan is discussed |
| What a word means | `docs/glossary.md` |
| How Fini writes: STE, the check of each word, how names are formed | `docs/naming.md` |
| Plans of past work | `docs/plans/` |

A README or a companion spec next to the code is an older form. When you
change one, move its content to the right place in `docs/`, delete it from the
code tree, and update the links to it. Do not start new docs next to code.

## How to write

- Write in STE (ASD-STE100): read `docs/naming.md` first. Use short sentences,
  one instruction in each sentence, the active voice, and approved words. A
  technical noun or verb that is not in the STE dictionary must be in
  `docs/glossary.md`.
- One place for each topic. Other documents link to it; they do not repeat it.
- Write what the code does now. Put what is not done yet in a short "Not done
  yet" section, not between the facts.
- Name code by its real names (types, functions, files), so that a reader can
  find it.
- Keep docs simple and quick to read. Write only the critical, high-level
  architecture: the parts, what each part owns, and how the parts connect.
  The docs must be sufficient to write the code again to that architecture.
  Leave out what a developer or an agent knows already.
- Put the explanation in the docs, and keep code comments short. A comment
  can link to the doc section.
- Write each sentence so that it has only one meaning. This is the reason
  that Fini uses STE.
- Show architecture as an ASCII tree, with each part below the part that
  holds it. A tree is quick to read.
- Add a mermaid diagram only for a complex algorithm that a tree cannot
  show. Keep to the number of new diagrams that the plan or the person gives.

## Approval

- A new document file needs the approval of the project owner first: give
  its name and its job.
- A new term in the glossary needs the approval of the project owner first.
- Changes to an ADR happen only while you discuss a plan with the project
  owner.

## Check

- Every link in the changed documents goes to a file that exists.
- No document repeats a topic that has its own place.
