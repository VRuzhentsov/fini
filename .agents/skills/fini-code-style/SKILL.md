---
name: fini-code-style
description: "Fini repo-wide code style conventions that apply across both the Rust backend and the Vue/TypeScript frontend. Load when writing or reviewing code for naming, constants, and other structural style choices, independent of which domain skill (fini-frontend, fini-dev-db, etc.) governs the surface being touched."
---

# Fini Code Style

Repo-wide style rules that apply regardless of which domain skill is driving the actual work. Load alongside the relevant domain skill (`fini-frontend`, `fini-dev-db`, etc.) rather than in place of it.

## No Magic Strings

When a value is compared, branched on, or reused more than once, give it a name instead of repeating the literal.

Avoid:

```ts
const result = ref<"found_enabled" | "found_not_enabled" | "not_found" | null>(null);
// ...
result.value = "found_enabled";
// ...
switch (result.value) {
  case "found_enabled":
    return "...";
}
```

Prefer a named constant (TypeScript) or a proper enum (Rust), and reference that name everywhere the value is produced or matched:

```ts
const FIND_RESULT = {
  FOUND_ENABLED: "found_enabled",
  FOUND_NOT_ENABLED: "found_not_enabled",
  NOT_FOUND: "not_found",
} as const;
type FindResult = (typeof FIND_RESULT)[keyof typeof FIND_RESULT] | null;
const result = ref<FindResult>(null);
// ...
result.value = FIND_RESULT.FOUND_ENABLED;
// ...
switch (result.value) {
  case FIND_RESULT.FOUND_ENABLED:
    return "...";
}
```

```rust
enum FindResult {
    FoundEnabled,
    FoundNotEnabled,
    NotFound,
}
```

Rules:

- A value used in more than one place (an assignment plus a comparison, or two-or-more comparisons) gets a name. A value used exactly once, inline, in an obviously self-explanatory spot (a single UI label, a single log message) does not need one.
- In TypeScript, prefer a `const ... as const` object with a derived union type (as above) over a plain string-literal union with no backing constant, and over a TypeScript `enum` (numeric/string enums have their own well-known footguns and aren't the idiomatic modern-TS pattern here).
- In Rust, prefer a real `enum` over a `&str`/`String` comparison whenever the set of values is closed and known at compile time.
- **Exception — mirroring an existing wire-format tag is not a magic string.** When TypeScript code matches a string exactly because it's mirroring a Rust `#[serde(tag = "...", rename_all = "snake_case")]` enum's wire representation (e.g. `TransportStatusCode` in `src/stores/device.ts`, matched against `status.state.code?.code === "connecting"`), the string literal *is* the contract — the discriminated-union-of-string-literal-tags pattern already established for that mirror is correct and expected. Do not introduce a parallel constants file for values that already have a canonical source of truth on the Rust side; that would create two things that must be kept in sync instead of one.
- When fixing a magic string, scope the fix to the specific value(s) actually in front of you. Do not sweep the rest of the file or codebase for other occurrences unless asked — matches `fini-dev`'s "make the smallest correct change."

## Review Checklist

Before handing off a change:

- No repeated string/numeric literal stands in for what should be a named constant or enum variant.
- A value that mirrors an existing backend wire-format tag was left as a literal (per the exception above), not needlessly wrapped in a new constant.
