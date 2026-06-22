# Theme Token Contract

Issue: #5

## Goal

Make Fini theme-ready by defining a token JSON contract that maps to CSS
variables at runtime. This contract supports built-in themes now and future
user-installed theme packs later, without adding theme import UI in this slice.

## Scope

- Theme values are represented as structured JSON tokens.
- Tokens map to CSS custom properties consumed by Vue components and global
  styles.
- Typography tokens are part of the contract.
- Theme scope is app-wide.
- Active theme preference is stored per device and must not sync through
  paired-device replication.
- Motion values remain fixed for now and are not tokenized.

## Token Shape

A theme token file must contain:

```json
{
  "id": "fini-default",
  "name": "Fini Default",
  "version": 1,
  "modes": {
    "light": {
      "color": {},
      "typography": {},
      "spacing": {},
      "radius": {},
      "shadow": {}
    },
    "dark": {
      "color": {},
      "typography": {},
      "spacing": {},
      "radius": {},
      "shadow": {}
    }
  }
}
```

Required top-level fields:

- `id`: stable machine-readable theme id.
- `name`: human-readable display name.
- `version`: integer schema version.
- `modes.light`: tokens for light rendering.
- `modes.dark`: tokens for dark rendering.

The runtime may reject a theme when required top-level fields or required mode
objects are missing.

## Token Categories

### Color

Color tokens describe semantic roles rather than component names:

- `page.bg`
- `border.soft`
- `border.softer`
- `fg.1` through `fg.6`
- `space.personal`
- `space.family`
- `space.work`
- DaisyUI bridge colors used by existing UI surfaces:
  - `base.100`
  - `base.200`
  - `base.300`
  - `base.content`
  - `primary`
  - `primary.content`
  - `success`
  - `warning`
  - `error`

### Typography

Typography tokens define:

- `family.sans`
- `family.mono`
- `size.body`
- `size.small`
- `size.title`
- `weight.regular`
- `weight.medium`
- `weight.semibold`
- `weight.bold`
- `lineHeight.body`
- `lineHeight.compact`

Components should consume typography through CSS variables or shared utility
classes rather than hard-coded font stacks when a token exists.

### Spacing

Spacing tokens define app-level rhythm values:

- `spacing.1`
- `spacing.2`
- `spacing.3`
- `spacing.4`
- `spacing.6`
- `spacing.8`

This does not replace all Tailwind spacing utilities in one change. New shared
surfaces should prefer token-backed variables when the spacing is semantic or
repeated across components.

### Radius

Radius tokens define:

- `radius.sm`
- `radius.md`
- `radius.lg`
- `radius.xl`
- `radius.2xl`

### Shadow

Shadow tokens define:

- `shadow.sm`
- `shadow.lg`

## CSS Variable Mapping

Token keys map to CSS custom properties by joining path segments with `-` and
prefixing them with `--theme-`.

Examples:

| Token path | CSS variable |
|---|---|
| `color.page.bg` | `--theme-color-page-bg` |
| `color.fg.1` | `--theme-color-fg-1` |
| `color.base.100` | `--theme-color-base-100` |
| `color.base.content` | `--theme-color-base-content` |
| `color.primary.content` | `--theme-color-primary-content` |
| `typography.family.sans` | `--theme-typography-family-sans` |
| `radius.md` | `--theme-radius-md` |

Existing Fini variables may bridge to theme variables during migration:

```css
:root {
  --color-page-bg: var(--theme-color-page-bg);
  --fg-1: var(--theme-color-fg-1);
  --color-base-100: var(--theme-color-base-100);
  --color-base-200: var(--theme-color-base-200);
  --color-base-300: var(--theme-color-base-300);
  --color-base-content: var(--theme-color-base-content);
  --color-primary: var(--theme-color-primary);
  --color-primary-content: var(--theme-color-primary-content);
  --color-success: var(--theme-color-success);
  --color-warning: var(--theme-color-warning);
  --color-error: var(--theme-color-error);
  --radius-md: var(--theme-radius-md);
}
```

This bridge lets components migrate incrementally without changing every
surface in the first implementation PR.

## Runtime Selection

The existing theme mode setting remains responsible for selecting `system`,
`light`, or `dark`.

The token runtime chooses the token mode after resolving system preference:

1. Resolve theme mode to `light` or `dark`.
2. Load the active app-wide theme token set.
3. Apply mode-specific tokens to `document.documentElement`.
4. Keep `data-theme` and native window theme behavior aligned with the
   resolved mode.

## Persistence

Theme preference is per-device local state.

- The selected theme mode stays in the local `settings` table.
- Future selected theme id should also be local-only.
- Theme preference must not be included in SpaceSync replication payloads.

## Invalid Or Incomplete Themes

When a token file is missing required fields or contains invalid values, the
runtime should fall back to the built-in default theme and keep the app usable.

The failure should be observable in development logs or test assertions, but it
should not block startup.

## Out Of Scope

- Marketplace or import UI for third-party themes.
- Per-space or per-view theme switching.
- Accessibility scoring gate enforcement for theme values.
- Tokenizing motion values.
- Replacing every current Tailwind or DaisyUI utility in one migration.

## Acceptance Criteria

- The default theme can be represented by the token schema.
- The schema includes color, surface, typography, spacing, radius, and shadow
  tokens.
- The mapping from token paths to CSS custom properties is deterministic.
- The bridge strategy supports incremental migration from current global CSS
  variables.
- Theme preference remains per-device local state.
