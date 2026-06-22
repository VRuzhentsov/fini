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
- Theme packs may optionally declare semantic icon mappings for future icon
  customization.
- Theme scope is app-wide.
- Active theme preference is stored per device and must not sync through
  paired-device replication.
- Motion values remain fixed for now and are not tokenized.

## Token Shape

A theme token file has this shape:

```json
{
  "id": "fini-default",
  "name": "Fini Default",
  "version": 1,
  "icons": {
    "set": "heroicons-outline-24",
    "overrides": {}
  },
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

Optional top-level fields:

- `icons`: icon asset mapping for semantic icon slots.

The runtime may reject a theme when required top-level fields or required mode
objects are missing.

## Token Leaves

Version 1 token leaves use primitive JSON values, not object wrappers. Theme
authors must set each token path directly to the CSS value that will be assigned
to the mapped custom property.

```json
{
  "modes": {
    "light": {
      "color": {
        "page": {
          "bg": "#f8fafc"
        },
        "base": {
          "content": {
            "channel": "27.8078% 0.029596 256.848"
          }
        }
      },
      "typography": {
        "family": {
          "sans": "Inter, ui-sans-serif, system-ui, sans-serif"
        },
        "size": {
          "body": "1rem"
        },
        "weight": {
          "semibold": 600
        }
      },
      "spacing": {
        "4": "1rem"
      },
      "radius": {
        "md": "0.5rem"
      },
      "shadow": {
        "sm": "0 1px 2px 0 rgb(15 23 42 / 0.08)"
      }
    }
  }
}
```

Accepted leaf value forms:

- Color, typography family, typography size, line height, spacing, radius, and
  shadow tokens: strings containing complete CSS property values.
- Typography weight tokens: numbers or strings that are valid CSS
  `font-weight` values.
- OKLCH `.channel` color tokens: strings containing channel tuples only,
  without the wrapping `oklch(...)` function.

Object leaves such as `{ "value": "#f8fafc" }` are not part of the version 1
contract. A future schema version may add token metadata, but the runtime for
version 1 should reject object leaves instead of silently accepting a different
shape.

## Token Categories

### Color

Color tokens describe semantic roles rather than component names:

- `page.bg`
- `border.soft`
- `border.softer`
- `fg.1` through `fg.6`
- `space.personal`
- `space.personal.content`
- `space.family`
- `space.family.content`
- `space.work`
- `space.work.content`
- DaisyUI bridge colors used by existing UI surfaces:
  - `base.100`
  - `base.200`
  - `base.300`
  - `base.content`
  - `base.content.channel`
  - `primary`
  - `primary.content`
  - `secondary`
  - `secondary.content`
  - `success`
  - `success.content`
  - `success.channel`
  - `warning`
  - `warning.content`
  - `warning.channel`
  - `error`
  - `error.content`
  - `error.channel`

Color tokens without a `.channel` suffix must be complete CSS color values.
The `.channel` tokens must be OKLCH channel tuples suitable for consumers that
wrap them in `oklch(var(...))`, such as priority rings. Keeping full colors and
channel tuples separate avoids one token value needing to satisfy incompatible
CSS syntaxes.

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

## Icon Assets

Icon mappings are optional and are not mode-specific. They describe semantic UI
slots rather than component filenames, so the app can keep stable defaults when
a theme does not customize icons.

```json
{
  "icons": {
    "set": "heroicons-outline-24",
    "overrides": {
      "quest.complete": "check",
      "quest.focus": "bolt",
      "quest.move": "arrows-right-left",
      "quest.delete": "trash",
      "reminder.due": "calendar-days",
      "space.personal": "user",
      "space.family": "home",
      "space.work": "briefcase"
    }
  }
}
```

Icon fields:

- `icons.set`: approved built-in icon set used by the mapping.
- `icons.overrides`: semantic slot to icon-name mapping.

For version 1, icon values should resolve to known icons from approved built-in
sets. Theme JSON should not contain arbitrary remote image URLs or raw SVG
markup. Future theme-pack import work may add bundled local SVG assets, with
validation and sanitization, without changing the color token contract.

When an icon slot is missing or an icon name is unsupported, the runtime should
fall back to the app default for that slot.

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
| `color.base.content.channel` | `--theme-color-base-content-channel` |
| `color.primary.content` | `--theme-color-primary-content` |
| `color.secondary.content` | `--theme-color-secondary-content` |
| `color.success.channel` | `--theme-color-success-channel` |
| `typography.family.sans` | `--theme-typography-family-sans` |
| `radius.md` | `--theme-radius-md` |

Existing Fini variables may bridge to theme variables during migration:

```css
:root {
  --color-page-bg: var(--theme-color-page-bg);
  --color-border-soft: var(--theme-color-border-soft);
  --color-border-softer: var(--theme-color-border-softer);
  --fg-1: var(--theme-color-fg-1);
  --fg-2: var(--theme-color-fg-2);
  --fg-3: var(--theme-color-fg-3);
  --fg-4: var(--theme-color-fg-4);
  --fg-5: var(--theme-color-fg-5);
  --fg-6: var(--theme-color-fg-6);
  --space-color-personal: var(--theme-color-space-personal);
  --space-color-personal-content: var(--theme-color-space-personal-content);
  --space-color-family: var(--theme-color-space-family);
  --space-color-family-content: var(--theme-color-space-family-content);
  --space-color-work: var(--theme-color-space-work);
  --space-color-work-content: var(--theme-color-space-work-content);
  --color-base-100: var(--theme-color-base-100);
  --color-base-200: var(--theme-color-base-200);
  --color-base-300: var(--theme-color-base-300);
  --color-base-content: var(--theme-color-base-content);
  --color-base-content-channel: var(--theme-color-base-content-channel);
  --color-primary: var(--theme-color-primary);
  --color-primary-content: var(--theme-color-primary-content);
  --color-secondary: var(--theme-color-secondary);
  --color-secondary-content: var(--theme-color-secondary-content);
  --color-success: var(--theme-color-success);
  --color-success-content: var(--theme-color-success-content);
  --color-success-channel: var(--theme-color-success-channel);
  --color-warning: var(--theme-color-warning);
  --color-warning-content: var(--theme-color-warning-content);
  --color-warning-channel: var(--theme-color-warning-channel);
  --color-error: var(--theme-color-error);
  --color-error-content: var(--theme-color-error-content);
  --color-error-channel: var(--theme-color-error-channel);
  --radius-sm: var(--theme-radius-sm);
  --radius-md: var(--theme-radius-md);
  --radius-lg: var(--theme-radius-lg);
  --radius-xl: var(--theme-radius-xl);
  --radius-2xl: var(--theme-radius-2xl);
  --shadow-sm: var(--theme-shadow-sm);
  --shadow-lg: var(--theme-shadow-lg);
}
```

This bridge lets components migrate incrementally without changing every
surface in the first implementation PR.

Existing filled space badges should bridge foreground colors as well as
backgrounds:

```css
.badge.space-color-personal {
  background-color: var(--space-color-personal);
  border-color: var(--space-color-personal);
  color: var(--space-color-personal-content);
}
```

Current priority UI that needs OKLCH channel values should migrate from
`oklch(var(--color-base-content)/0.3)`, `oklch(var(--color-success))`,
`oklch(var(--color-warning))`, and `oklch(var(--color-error))` to the
corresponding `--color-*-channel` bridge variables. Direct surfaces should keep
using the complete color variables such as `--color-base-content`,
`--color-success`, `--color-warning`, and `--color-error`.

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
