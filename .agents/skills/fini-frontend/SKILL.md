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

### Single named handler per event binding

Avoid branching logic inline in an event binding such as:

```vue
@click="
  isRetryable(status)
    ? void retry()
    : status.state !== 'unconfigured' && enabled === null && void pin(status.kind)
"
```

Instead, expose one named function per binding and put every branch inside it:

```ts
function handleRowClick(status: RowStatus) {
  if (isRetryable(status)) {
    void retry();
    return;
  }
  if (status.state !== "unconfigured" && enabled.value === null) {
    void pin(status.kind);
  }
}
```

```vue
@click="handleRowClick(status)"
```

The same applies to a dynamic prop that's computed from more than one condition (e.g. `:button`, `:disabled` guarding more than a simple two-term boolean, `:class` picking between named states) — give it a named function (`isRowClickable(status)`, `rowActionLabel(status)`) rather than inlining the `&&`/`?:`/multi-term expression in the template. A single two-term `:disabled="a || b"` is fine inline; branching to different function calls or building a multi-part label is not.

Rules:

- One `@click`/`@submit`/etc. binding calls exactly one named function; that function contains all the branching.
- A prop bound from more than one condition, or from a condition mixed with a function call, gets a named function too.
- Inside `<script setup>`, refs need `.value` explicitly — unlike the template's auto-unwrap, a plain `ref` read in one of these handler/computed functions without `.value` is a real bug, not a style nit.

## Component Extraction

When adding a new template block that exceeds roughly ten lines of HTML, first extract reusable semantic controls (for example, the Energy and Priority selectors shared by create and edit views) into focused child components instead of expanding the parent view. Do not split one coherent form section into a generic `*Details` wrapper merely to meet the line threshold; keep its local layout with the owning form when that is clearer.

Keep the parent responsible for form/lifecycle orchestration and pass narrow state and events to shared controls. Every extracted component must still follow these `renderFlags` rules for its own conditional sections. Do not use extraction to hide direct conditional-rendering logic from review.

## Testing Expectations

When adding or changing render flags:

- Add or update component tests for both visible and hidden states when the condition is product/platform behavior.
- Prefer assertions on user-visible labels or test IDs, not internal computed names.
- Include at least one test that would fail if the section rendered unconditionally.

## Review Checklist

Before handing off frontend template changes, check:

- Template conditionals use `renderFlags` for product/platform rendering decisions.
- Non-trivial list rendering uses a named computed list source.
- Event bindings call exactly one named function; branching logic lives in that function, not the template.
- Tests cover important visible and hidden render states.
- `npm run build` or the relevant frontend test target passes.
