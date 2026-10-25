---
name: fini-code-style
description: "Fini repo-wide code style rules (naming, constants, structural conventions) that apply across the Rust backend and Vue/TypeScript frontend, independent of which domain skill governs the surface being touched. Deliberately terse -- grows over time, one short rule at a time."
---

# Fini Code Style

Repo-wide rules, independent of domain. Load alongside the relevant domain skill (`fini-frontend`, `fini-dev-db`, etc.), not instead of it.

Kept terse on purpose: this file grows over time. Each rule stays one short paragraph — state the rule and its one real exception, skip code-block examples unless a rule is genuinely ambiguous without one.

## Rules

**Check every word of a new name in the STE dictionary before you write it.**
Do the steps in `docs/ste.md`, "Check a name before you write it", for each
new file, type, function, method, field, constant, command and test hook, and
for each new comment and log message. Look up a word with
`grep -E "^(WORD|word) \(" ~/.cache/agents/ste/ste.txt`: an uppercase entry
is approved, a lowercase entry gives the approved word to use in its place,
and no entry means not approved. Keep glossary terms and the names of the
language and of libraries. Before you report the work, read your diff once
more for the words in the glossary's "Words deliberately not used" list.

**Names come from `docs/glossary.md` and `docs/naming.md`.** The glossary
holds the vocabulary — channel (transport), channel kind, DataLink, session,
primary — and settles *which* word. `naming.md` holds how names are formed —
plurals, case per language, `data-*` hooks, Tauri command prefixes — and
settles how to spell it. Read the relevant one before inventing a name,
renaming one, or arguing about one; don't restate their rules here.

**No magic strings.** A closed set of values — states, kinds, colours, problem codes, frame types — is named once and used through that name everywhere: a `const ChannelColor = { Green: "green", … } as const` object with `type ChannelColor = (typeof ChannelColor)[keyof typeof ChannelColor]` in TypeScript, an `enum` in Rust. Comparisons, `switch`es, props, test fixtures and e2e helpers all write `ChannelColor.Green`, never `"green"`. A union type alone does not satisfy this — `color === "green"` against `"green" | "grey"` is still a magic string, just a type-checked one. Mirroring a Rust `#[serde]` wire format is not an exception: the const object is the one place the wire literals are spelled, and everything else goes through it. Keep such consts in a module with no runtime imports (no Tauri, no Pinia) so specs and e2e helpers import them rather than redeclare them. In Rust, match on the enum; never compare its `.code()` or serialized string. Only a lone value with no set behind it, used once in a self-explanatory spot, may stay inline. When fixing one, fix the whole set in front of you, not a codebase sweep.
