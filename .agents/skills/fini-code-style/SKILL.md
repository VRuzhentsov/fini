---
name: fini-code-style
description: "Fini repo-wide code style rules (naming, constants, structural conventions) that apply across the Rust backend and Vue/TypeScript frontend, independent of which domain skill governs the surface being touched. Deliberately terse -- grows over time, one short rule at a time."
---

# Fini Code Style

Repo-wide rules, independent of domain. Load alongside the relevant domain skill (`fini-frontend`, `fini-dev-db`, etc.), not instead of it.

Kept terse on purpose: this file grows over time. Each rule stays one short paragraph — state the rule and its one real exception, skip code-block examples unless a rule is genuinely ambiguous without one.

## Rules

**Shape the code as `docs/architecture/code-design.md` says.** Read it before
you write or change code, and do its steps in "Before you write code" for each
change. Objects own their state and behavior; a behavior that every
implementation has goes on the interface; use and change the code that does
the job already; remove what your change makes unused. Read your diff for
these rules before you report the work.

**Check every word of a new name in the STE dictionary before you write it.**
Do the steps in `docs/naming.md`, "Check a name before you write it", for each
new file, type, function, method, field, constant, command and test hook, and
for each new comment and log message. In the dictionary, an uppercase entry
is approved, a lowercase entry gives the approved word to use in its place,
and no entry means not approved. Keep glossary terms and the names of the
language and of libraries. Before you report the work, read your diff once
more for the words in the glossary's "Words deliberately not used" list.

**Names come from `docs/glossary.md` and `docs/naming.md`.** The glossary
holds the vocabulary and settles *which* word. `naming.md` holds STE, the
dictionary check, and how names are formed, and settles how to write it. Read the relevant one before inventing a name,
renaming one, or arguing about one; don't restate their rules here.

**Compute derived state; do not generate or store it ahead.** A value that
follows from stored data is computed when it is read: a Vue `computed` getter in
the frontend, a method over the stored rows in Rust. Store only what a person
did (ADR-0012: a repeating quest's occurrences are computed from its rule; only
the days a person closed or changed have a row). Nothing appears in the
database only because time passed or a record exists. The exception is a
value whose computation is too slow to repeat on each read; store it then as a
cache with the rule that rebuilds it.

**Declare a value; do not build it step by step.** Write a list, a menu or a
set of options as one literal that shows all of it, and give each entry the
condition that makes it apply (for example a `visible` field on a menu item).
Do not start from an empty value and add parts with `push` and `if` branches,
and do not name a function `build…` or `make…`. In Vue, the literal is a
`computed` in a composable (`useQuestMenu`: the whole quest menu, each item
with `visible`); the code that shows it drops the entries that do not apply.
The exception is a value that comes from a loop over data (one menu item for
each space): use `map` or `filter` over the data for that part.

**No magic strings.** A closed set of values — states, kinds, colours, problem codes, frame types — is named once and used through that name everywhere: a `const ChannelColor = { Green: "green", … } as const` object with `type ChannelColor = (typeof ChannelColor)[keyof typeof ChannelColor]` in TypeScript, an `enum` in Rust. Comparisons, `switch`es, props, test fixtures and e2e helpers all write `ChannelColor.Green`, never `"green"`. A union type alone does not satisfy this — `color === "green"` against `"green" | "grey"` is still a magic string, just a type-checked one. Mirroring a Rust `#[serde]` wire format is not an exception: the const object is the one place the wire literals are spelled, and everything else goes through it. Keep such consts in a module with no runtime imports (no Tauri, no Pinia) so specs and e2e helpers import them rather than redeclare them. In Rust, match on the enum; never compare its `.code()` or serialized string. Only a lone value with no set behind it, used once in a self-explanatory spot, may stay inline. When fixing one, fix the whole set in front of you, not a codebase sweep.
