---
name: fini-release-prep
description: Use this skill for Fini major release preparation, especially when the user asks for Play Store screenshots, Android marketplace screenshots, store listing assets, release screenshot packages, or "major release prep". This skill should trigger even if the user only mentions screenshots, because Fini's canonical store assets live in `docs/play-store/` and must be generated, checked, and reported consistently before a release.
---

# Fini Release Prep

Use this skill to prepare marketplace-facing release assets for Fini. The first supported workflow is the Google Play screenshot package.

## Outcome

Produce a repeatable Play Store screenshot package:

- normal Fini app runtime, scaled to mobile/tablet viewport proportions
- curated demo data only, never the user's local data
- core product flow screenshots
- store-ready artwork with stable captions/value props
- light and dark variants where the UI supports both
- canonical output under `docs/play-store/screenshots/`
- machine-readable manifest for upload/review

## Required Skills

Load these before doing the actual work:

- `fini-dev` first, for repo workflow and verification rules.
- `fini-scripting` before changing Makefile, npm scripts, `xtask`, or generated-asset automation.
- `browse` or `gstack` when driving a web/runtime viewport for screenshots.
- `fini-android-testing` only when the user specifically wants real Android device/emulator evidence.
- `fini-versioning` only if release prep expands into version bumps or release tags.

## Canonical Paths

Use these paths by default:

- listing text: `docs/play-store/listing.md`
- feature graphic: `docs/play-store/feature-graphic.png`
- phone screenshots: `docs/play-store/screenshots/phone/`
- 7-inch tablet screenshots: `docs/play-store/screenshots/tablet-7/`
- 10-inch tablet screenshots: `docs/play-store/screenshots/tablet-10/`
- manifest: `docs/play-store/screenshots/manifest.json`

Do not create timestamped release folders unless the user asks for review sets. The agreed default is to overwrite/update the canonical asset tree.

## Screenshot Matrix

Maintain this first-version matrix:

| Device bucket | Target size | Folder |
|---|---:|---|
| Phone | `780x1387` | `docs/play-store/screenshots/phone/` |
| 7-inch tablet | `1200x1920` | `docs/play-store/screenshots/tablet-7/` |
| 10-inch tablet | `1600x2560` | `docs/play-store/screenshots/tablet-10/` |

Generate or verify these core shots for each device bucket:

| File | Surface | Caption intent |
|---|---|---|
| `01-focus.png` | Focus / active quest | One quest at a time |
| `02-history.png` | History | Finish or abandon without pile-up |
| `03-settings.png` | Settings / privacy proof | Local-first and private |

When adding light/dark variants, use explicit suffixes such as `01-focus-light.png` and `01-focus-dark.png`, then update `manifest.json` to list both variants.

## Workflow

1. Read `docs/play-store/listing.md` to keep captions aligned with the store positioning.
2. Confirm the app state uses curated demo data. If the current runtime may contain personal data, stop and switch to an isolated `FINI_APP_DATA_DIR` under `/var/tmp`.
3. Drive the normal Fini app/runtime at the target viewport dimensions. Use the web/runtime path unless the user explicitly asks for Android device screenshots.
4. Capture the core product flow in this order: Focus, History, Settings. Expand only if the user asks for more store panels.
5. Compose store-ready artwork with stable captions when composition tooling is available. Preserve the product UI as the main visual; captions explain the value prop, not fictional functionality.
6. Write outputs into the canonical folders above.
7. Run `make play-store-screenshots` to regenerate the manifest and validate required files/dimensions.
8. Report exact asset paths, manifest path, command output, and any missing manual visual review.

## Make Target

Use this command after capture/composition:

```bash
make play-store-screenshots
```

The target validates the canonical screenshot package and writes `docs/play-store/screenshots/manifest.json`. Treat failures as release blockers because missing or wrongly sized assets waste store-review time.

## Reporting

Final response should include:

- screenshots generated or validated, with folder paths
- manifest path
- command evidence from `make play-store-screenshots`
- visual review status, including whether light/dark variants were inspected
- deferred work, especially if real Android device screenshots or alternate-market sizes were not requested
