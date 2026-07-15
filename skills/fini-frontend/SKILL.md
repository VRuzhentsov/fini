---
name: fini-frontend
description: "Fini Vue frontend implementation conventions for views, templates, rendering decisions, and tests."
---

# Fini Frontend Workflow

Use this skill when creating or changing Vue frontend code under `src/`, especially view components, templates, conditional rendering, lists, and frontend tests.

## Template Rendering Rules

### Centralize render decisions in `renderFlags`

Avoid embedding render decision logic directly in Vue templates with ad hoc expressions such as:

```vue
<section v-if="startupAutoUpdateSupported && !loading">
```

Instead, expose a computed `renderFlags` object from the component and bind template conditionals to named flags:

```ts
const renderFlags = computed(() => ({
  automaticUpdatesSection: startupAutoUpdateSupported.value,
}));
```

```vue
<section v-if="renderFlags.automaticUpdatesSection">
```

Rules:

- Every non-trivial `v-if`, `v-show`, or conditional template section should use a named `renderFlags` key.
- `renderFlags` is not the source of all component state. It is only the template render contract: each key answers whether a specific UI section or element should render.
- Keep domain state, loading state, form state, selected entities, fetched data, and user input in their normal refs, stores, or computed values outside `renderFlags`.
- Let `renderFlags` derive from those state sources instead of replacing them.
- Each key should describe the UI section or element being rendered, not the low-level implementation detail.
- Keep product/platform render logic in the computed flag, not in the template.
- Prefer names like `automaticUpdatesSection`, `emptyState`, `deviceList`, or `restoreNotice` over names like `isDesktopAndEnabled`.
- Simple local DOM-only toggles may stay inline only when the condition is self-evident and not product/platform logic.

### Centralize list sources for `v-for`

For non-trivial lists, avoid filtering, sorting, or mapping directly inside `v-for`.

Prefer a named computed list source:

```ts
const renderLists = computed(() => ({
  visibleDevices: devices.value.filter((device) => device.visible),
}));
```

```vue
<DeviceRow
  v-for="device in renderLists.visibleDevices"
  :key="device.id"
  :device="device"
/>
```

Rules:

- `v-for` should normally iterate over a named source that is already filtered and ordered.
- Keep data shaping and eligibility logic outside the template.
- Use stable keys derived from domain IDs when available.

## Testing Expectations

When adding or changing render flags:

- Add or update component tests for both visible and hidden states when the condition is product/platform behavior.
- Prefer assertions on user-visible labels or test IDs, not internal computed names.
- Include at least one test that would fail if the section rendered unconditionally.

## Review Checklist

Before handing off frontend template changes, check:

- Template conditionals use `renderFlags` for product/platform rendering decisions.
- Non-trivial list rendering uses a named computed list source.
- Tests cover important visible and hidden render states.
- `npm run build` or the relevant frontend test target passes.
