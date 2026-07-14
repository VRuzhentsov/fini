# Release Notes

Fini publishes concise, user-facing release notes from the commits between the previous tag and the release tag. The release workflow preserves GitHub's generated PR list and full changelog below the curated notes.

## Format

The structure follows the clear, product-surface-first form used by OpenCode releases:

```md
## Core

### New
- Add a focused capability users can use now.

### Improvements
- Improve an existing workflow or reliability.

### Bugfixes
- Fix a user-visible defect.

## Desktop

### Bugfixes
- Fix a desktop-specific issue.
```

The generator omits empty sections and only reports user-facing changes. It groups entries into `Core`, `Desktop`, `CLI`, `Android`, or `Distribution`.

## Minor releases

Minor releases (`v0.MINOR.0`) lead with new capabilities, then improvements and bug fixes. Use `feat(<area>): ...` for every user-facing capability so it is placed under `### New`.

## Patch releases

Patch releases (`v0.MINOR.PATCH`) lead with `### Bugfixes`, followed by `### Improvements`. A feature commit in a patch release is presented as an improvement; prefer a minor release when it is a material new capability.

## How entries are collected

`cargo xtask release-notes <previous-tag> <release-tag> <output-file>` reads the commit subjects between tags. Conventional Commit subjects give deterministic categories; existing plain-language squash-merge titles are also included with a small verb-based classification so an older PR is not silently lost.

| Commit type | Release-note section |
|---|---|
| `feat` | New (or Improvements for a patch release) |
| `fix` | Bugfixes |
| `perf`, `refactor`, other user-facing types | Improvements |
| `build`, `chore`, `ci`, `docs`, `release`, `test` | Omitted |

The optional scope determines the product surface:

| Scope contains | Surface |
|---|---|
| `android` | Android |
| `cli` | CLI |
| `desktop`, `app`, `settings`, `ui`, `updater` | Desktop |
| `release`, `package`, `linux`, `windows` | Distribution |
| anything else | Core |

Write the subject in plain language for users, not implementation jargon. Include the PR number in the normal `(#123)` suffix; it is removed from the curated bullet because GitHub appends its generated PR list and compare link.

## Release workflow

The stable and prerelease publication jobs fetch tag history, generate `release-notes.md`, then call `gh release create --notes-file release-notes.md --generate-notes`. Do not hand-edit a published release as the normal path: improve the originating PR title or commit subject before tagging so the published notes remain reproducible.
