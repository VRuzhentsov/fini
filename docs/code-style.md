# Code style

These rules apply to new and changed code in all parts of the app: the Rust
backend and the Vue/TypeScript frontend. How code is shaped (objects,
interfaces, patterns) is in [`architecture/code-design.md`](architecture/code-design.md).
Names, comments, logs and documents also follow [`naming.md`](naming.md) and
the words in [`glossary.md`](glossary.md).

Each rule stays one short paragraph: the rule and its one real exception. Add
a code example only when a rule is ambiguous without one.

## Rules for all code

**Use object-oriented programming (OOP) as the default.** Put domain behavior
on its owning object: Rust structs or enums with `impl`, or TypeScript classes.
Put parsing and conversion on the value type or its codec, even when pure.
Use existing owners before adding types. Check each free helper against the
limited exceptions in `architecture/code-design.md` during code review.

**Shape the code as `architecture/code-design.md` says.** Read it before
you write or change code, and do its steps in "Before you write code" for each
change. Objects own their state and behavior; a behavior that every
implementation has goes on the interface; use and change the code that does
the job already; remove what your change makes unused. Read your diff for
these rules before you report the work.

**Check every word of a new name in the STE dictionary before you write it.**
Do the steps in `naming.md`, "Check a name before you write it", for each
new file, type, function, method, field, constant, command and test hook, and
for each new comment and log message. In the dictionary, an uppercase entry
is approved, a lowercase entry gives the approved word to use in its place,
and no entry means not approved. Keep glossary terms and the names of the
language and of libraries. Before you report the work, read your diff once
more for the words in the glossary's "Words deliberately not used" list.

**Names come from `glossary.md` and `naming.md`.** The glossary holds the
vocabulary and settles *which* word: what the words **mean**. `naming.md`
holds STE, the dictionary check, and how names are formed, and settles how to
write it. Read the relevant one before you name or rename a table, module,
type, command or DOM hook, and before you argue about a name. Either file wins
over what the code says now.

**Compute derived state; do not generate or store it ahead.** A value that
follows from stored data is computed when it is read: a Vue `computed` getter in
the frontend, a method over the stored rows in Rust. Store only what a person
did (ADR-0012: a repeating quest's occurrences are computed from its rule; only
the days a person closed or changed have a row). Nothing appears in the
database only because time passed or a record exists. The exception is a
value whose computation is too slow to repeat on each read; store it then as a
cache with the rule that rebuilds it.

**Name a function as STE writes text.** Use Command–Query Separation
(Bertrand Meyer; names as in the Swift API Design Guidelines). A query returns
a value and changes nothing. It is descriptive writing (STE Section 6): name it
with the technical noun phrase of the value (Section 2). A command changes
state. It is procedural writing (Section 5): name it with an approved verb in
the command form (rule 5.3). A function is a query or a command, not both.

**Declare a value.** Write a list, a menu or a set of options as one literal
that shows all of it. Each entry carries the condition that makes it apply.
For the part that comes from data, use `map` or `filter` over that data.

**No magic strings.** A closed set of values — states, kinds, colours, problem codes, frame types — is named once and used through that name everywhere: a `const ChannelColor = { Green: "green", … } as const` object with `type ChannelColor = (typeof ChannelColor)[keyof typeof ChannelColor]` in TypeScript, an `enum` in Rust. Comparisons, `switch`es, props, test fixtures and e2e helpers all write `ChannelColor.Green`, never `"green"`. A union type alone does not satisfy this — `color === "green"` against `"green" | "grey"` is still a magic string, just a type-checked one. Mirroring a Rust `#[serde]` wire format is not an exception: the const object is the one place the wire literals are spelled, and everything else goes through it. Keep such consts in a module with no runtime imports (no Tauri, no Pinia) so specs and e2e helpers import them rather than redeclare them. In Rust, match on the enum; never compare its `.code()` or serialized string. Only a lone value with no set behind it, used once in a self-explanatory spot, may stay inline. When fixing one, fix the whole set in front of you, not a codebase sweep.

**Reuse code before you add it.** Before adding a new constant, function,
type, or utility, search for an existing one:

- Check `src/stores/` for domain constants and helpers (e.g. `BUILTIN_SPACE_IDS`, `isBuiltinSpace` in `space.ts`).
- Check `src/utils/` for general-purpose utilities.
- Check `src/composables/` for shared Vue composables.
- Use `grep -r` or the Explore agent when the location is not obvious.

If an equivalent already exists, import and reuse it. Do not duplicate logic in
a view, component, or test that belongs in a store or utility. If the existing
name is slightly different but semantically identical, prefer renaming the call
site over adding a wrapper.

**Fix compiler warnings in the code.** Resolve compiler warnings by improving
code structure, feature boundaries, visibility, or imports. Treat broad
suppression such as crate-level `allow(dead_code, unused_imports)` as a
user-owned exception: add it only after explicit user approval, keep its scope
as narrow as possible, and call it out in the report.

## package.json scripts

**A script is a short name for one task.** Write one command, or a sequence of
`npm run <script>` calls, in fewer than 100 characters. Put the other parts here:

- Flags and options: in the config file of the tool (`vite.config.ts`,
  `specs/e2e/playwright.config.ts`, `jest.config.cjs`, `tsconfig.json`,
  `src-tauri/tauri.conf.json`).
- Environment variables, more than one step, `if`, `export` and
  `${VAR:-default}`: in a `scripts/*.mts` file that the script calls.
- Names: `<task>:<target>[:<variant>]`, as `test:e2e:smoke` and `build:android:e2e`.

Good: `"test:e2e": "node scripts/e2e-desktop.mts"`. Bad:
`"test:e2e": "FINI_APP_BINARY=$t/fini-app TZ=UTC npx playwright test --config … --project ui"`.

## Vue frontend

These rules apply to Vue code under `src/`: view components, templates,
conditional rendering, and lists.

### Styles: DaisyUI and Tailwind first

For Vue templates and CSS/class work, prefer the existing UI framework before writing local styles:

- Use DaisyUI component classes first when they express the control or surface.
- Use Tailwind utilities for layout, spacing, sizing, state, and small visual adjustments.
- Add scoped custom CSS/classes only when DaisyUI/Tailwind cannot produce a high-quality result, when a reusable semantic component class already exists locally, or when a narrow interaction/layout behavior would be unreadable as utilities.
- Keep custom CSS concise and justify it through the component/spec context; do not create parallel style systems, broad one-off class taxonomies, or copied prototype class names.

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

### No direct DOM queries

Never reach into the DOM by hand: no `document.querySelector`, `querySelectorAll`, `getElementById`, `getElementsBy*`, `element.closest(...)` lookups, or reading another component's markup through its classes or ids. This holds in components, stores and composables alike, with no exceptions.

Use the Vue way instead:

- An element this component renders: a template ref (`ref="menu"` + `useTemplateRef("menu")` or `const menu = ref<HTMLElement | null>(null)`).
- An element another component renders: that component exposes what is needed (`defineExpose`, an emitted size, a prop), or the shared value lives in a store / `provide`–`inject`.
- Size or position of something: measure it through its template ref (a `ResizeObserver` on the ref, or `@vueuse/core` helpers if already a dependency), and pass the number where it is needed.
- Focus, scroll, selection: call the method on the template ref.

Tests follow the same rule for app code; component specs use `wrapper.find(...)` from `@vue/test-utils`, and e2e uses Playwright locators — never `document.querySelector` inside the app.

### Component extraction

When adding a new template block that exceeds roughly ten lines of HTML, first extract reusable semantic controls (for example, the Energy and Priority selectors shared by create and edit views) into focused child components instead of expanding the parent view. Do not split one coherent form section into a generic `*Details` wrapper merely to meet the line threshold; keep its local layout with the owning form when that is clearer.

Keep the parent responsible for form/lifecycle orchestration and pass narrow state and events to shared controls. Every extracted component must still follow these `renderFlags` rules for its own conditional sections. Do not use extraction to hide direct conditional-rendering logic from review.

## Review checklist

- Each behavior has an owner. Free functions are limited to the exceptions in
  `architecture/code-design.md`.
- Each new name, comment and log message passes the STE check in `naming.md`.
- No value from a closed set (state, kind, colour, problem code) is written as
  a string literal; it goes through its named const.
- Derived state is computed when it is read, not stored.
- Each `package.json` script is one short command; flags go in tool config,
  environment variables and logic go in `scripts/*.mts`.
- No new code duplicates an existing store, utility or composable.
- Vue templates: conditionals use `renderFlags` for product/platform rendering
  decisions; non-trivial lists use a named computed list source; event
  bindings call exactly one named function; no `document.querySelector` or
  other direct DOM lookups (template refs, exposed APIs, props or stores
  instead).
- Tests cover important visible and hidden render states
  ([`testing.md`](testing.md), "Vue render states").
