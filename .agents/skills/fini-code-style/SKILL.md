---
name: fini-code-style
description: "Fini repo-wide code style rules (naming, constants, structural conventions) that apply across the Rust backend and Vue/TypeScript frontend, independent of which domain skill governs the surface being touched. Deliberately terse -- grows over time, one short rule at a time."
---

# Fini Code Style

Repo-wide rules, independent of domain. Load alongside the relevant domain skill (`fini-frontend`, `fini-dev-db`, etc.), not instead of it.

Kept terse on purpose: this file grows over time. Each rule stays one short paragraph — state the rule and its one real exception, skip code-block examples unless a rule is genuinely ambiguous without one.

## Rules

**No magic strings.** A value compared or reused more than once gets a name — a `const ... as const` object with a derived union type in TypeScript, a real `enum` in Rust — not a repeated literal. A value used exactly once, inline, in an obviously self-explanatory spot doesn't need one. Exception: a TS literal matched to mirror an existing Rust `#[serde(tag = "...")]` enum's wire format (e.g. `TransportStatusCode` in `src/stores/device.ts`) is not a magic string — that literal *is* the contract; don't wrap it in a parallel constant. When fixing one, scope the fix to the value in front of you, not a codebase sweep.
