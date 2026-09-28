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

One `@click`/`@submit`/etc. binding calls exactly one named function (`handleRowClick(status)`), with every branch inside that function — not a `?:`/`&&` chain inline in the template. The same applies to a dynamic prop computed from more than one condition (`:button`, `:disabled` beyond a simple two-term boolean, `:class` picking between named states): give it a named function too. A plain two-term `:disabled="a || b"` is fine inline. Inside `<script setup>`, remember refs need `.value` explicitly in these handler functions — the template's auto-unwrap doesn't apply there.

## No direct DOM queries

Never reach into the DOM by hand: no `document.querySelector`, `querySelectorAll`, `getElementById`, `getElementsBy*`, `element.closest(...)` lookups, or reading another component's markup through its classes or ids. This holds in components, stores and composables alike, with no exceptions.

Use the Vue way instead:

- An element this component renders: a template ref (`ref="menu"` + `useTemplateRef("menu")` or `const menu = ref<HTMLElement | null>(null)`).
- An element another component renders: that component exposes what is needed (`defineExpose`, an emitted size, a prop), or the shared value lives in a store / `provide`–`inject`.
- Size or position of something: measure it through its template ref (a `ResizeObserver` on the ref, or `@vueuse/core` helpers if already a dependency), and pass the number where it is needed.
- Focus, scroll, selection: call the method on the template ref.

Tests follow the same rule for app code; component specs use `wrapper.find(...)` from `@vue/test-utils`, and e2e uses Playwright locators — never `document.querySelector` inside the app.

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
- No `document.querySelector` or other direct DOM lookups; template refs, exposed APIs, props or stores instead.
- No value from a closed set (state, kind, colour, problem code) is written as a string literal — in components, stores, specs or e2e helpers it goes through its named const (`fini-code-style`).
- Tests cover important visible and hidden render states.
- `npm run build` or the relevant frontend test target passes.
