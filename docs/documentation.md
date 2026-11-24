# Documentation

How to write the Fini documents, and where each document is. Documents hold
the knowledge of the project; the skills in `.agents/skills/` hold only the
agent process and link to the documents.

## Where the rules are

- [`naming.md`](naming.md): the English that Fini uses (STE), the check of
  each word, and how to write a name.
- [`glossary.md`](glossary.md): what each word means.
- This file: where each kind of document goes, how to write it, and which
  changes need approval.
- The `dev` skill, "Use STE"
  ([`SKILL.md`](../.agents/skills/dev/SKILL.md)): the STE check of each
  diff before you commit.

## Where documents go

All documents are in `docs/`:

| Kind | Place |
|---|---|
| Architecture: how a part of the app works | `docs/architecture/<part>.md` (for example `communication.md`) |
| How code is shaped, in all parts of the app | `docs/architecture/code-design.md` |
| Code style rules, backend and frontend | [`code-style.md`](code-style.md) |
| Development guidelines: project map, design source, commands, repo automation, agent work | [`development.md`](development.md) |
| Agent plugin structure and source import | [`development.md`, Agent plugins](development.md#agent-plugins); imported contracts in [`src-agent-plugin/README.md`](../src-agent-plugin/README.md) |
| Test surfaces, test commands, how to write tests | [`testing.md`](testing.md); the e2e harness in [`e2e.md`](e2e.md) and [`e2e-android.md`](e2e-android.md) |
| Android development on a phone or emulator | [`android.md`](android.md) |
| Database access and schema migrations | [`db.md`](db.md) |
| CI targets | [`ci.md`](ci.md); the images in [`container.md`](container.md) |
| Release flow and version metadata | [`release.md`](release.md) |
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

## Rules

- Write a rule in general words. Do not name the incident that caused it.
- Do not add a condition that the rule does not need.
- Keep facts that code or config already holds in that code or config. The
  document links to it.
- Refer to the manifest (`Cargo.toml`, `package.json`) for versions. Keep
  versions, tags and commit hashes out of docs.
- Base each rule on a source or on a decision that was made. Do not invent
  rules.
- Put a rule in the document that owns its topic: development rules go in
  the development guidelines, not in a skill.
- Keep glossary entries to a short definition of the term; put examples and
  usage in the doc that covers the feature.

## Instructions for agents

When writing or editing any repo-local skill, `AGENTS.md`/`CLAUDE.md`, spec, or companion doc, describe the behavior an agent should follow, not the behavior it should avoid.

- Write instructions as affirmative, actionable defaults: "Do X", "Use Y", "Prefer Z" — not "Don't do X".
- State the expected behavior and default first, so the desired outcome is explicit before any exception.
- Pair a real limitation with what to do instead, in the same instruction, rather than leaving a bare prohibition.
- Reserve prohibition wording ("never", "do not") for hard safety, security, or compliance constraints — the kind of boundary that has no affirmative substitute.
- When reviewing or reformatting an existing instruction, prefer rewriting a standalone negative into its affirmative form over adding a second negative sentence next to it.
- Keep a skill to the agent process. Put the knowledge in `docs/` and link
  to it from the skill.

## Examples

- Add an example only when the reader cannot understand the rule from the
  text of the rule.
- Do not add an example that the reader does not need. It makes the document
  longer and gives no new information.

## Approval

- A new document file needs the approval of the project owner first: give
  its name and its job.
- A new term in the glossary needs the approval of the project owner first.
  Give each term an *Elsewhere* line: the nearest equivalent in other
  systems or designs.
- Changes to an ADR happen only while you discuss a plan with the project
  owner.
