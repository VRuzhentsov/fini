---
name: fini-design
description: Design and refine Fini interfaces in Figma with a Figma-first workflow. Use this whenever the user wants UI or UX design work, component cleanup, variant/property architecture, design-system refinement, visual polish, interaction-state design, or design QA in Figma. Use TalkToFigma for native design edits and ask the user to perform any Figma UI actions that are not available through automation tools. If a sibling `../fini-design/` design repo exists, treat it as a source of durable design references and exported assets.
---

# Fini Design

Use this skill for design work that happens primarily in Figma: creating or refining components, restructuring component sets, cleaning up variant properties, improving layout and spacing, aligning visual states with product behavior, and validating UI consistency before implementation.

## Sibling Design Repo

A sibling `../fini-design/` repo may exist alongside this project. When present, it acts as the durable home for design source material, exported assets, references, mockups, and design notes that should outlive any single Figma session.

Resolve the sibling path from the current repository name:

```text
current repo: <repo-name>
design repo: ../<repo-name>-design/
```

Example:

```text
repo: fini
design repo: ../fini-design/
```

Use the sibling design repo when:
- looking up prior design references, mockups, or exported assets
- locating canonical design documentation that the Figma file alone does not capture
- saving exported flows or static design artifacts that should be version-controlled

If `../fini-design/` does not exist, do not create it. Continue working from Figma alone and surface the missing path only if the user expects design material to live there.

## Outcome

Produce clean, maintainable design artifacts that are easy to evolve:
- Components have clear property models.
- Variants represent real behavioral axes rather than ad hoc examples.
- Nested pieces are reused from shared definitions.
- Typography, spacing, alignment, and color are internally consistent.
- Design decisions are verified in the live file before claiming completion.

## Tool Strategy

### Primary: TalkToFigma

Use `TalkToFigma_*` tools as the default path for design work inside the file.

Prefer TalkToFigma for:
- reading document, selection, and node metadata
- inspecting component sets, frames, text nodes, and instances
- editing fills, strokes, text, layout, spacing, sizing, corner radius, and annotations
- creating frames, text nodes, rectangles, component instances, and connections
- checking local components and instance overrides

The Figma file state exposed through TalkToFigma is the source of truth for native design structure.

### Fallback: User-Performed Figma Actions

Some Figma actions are not available through TalkToFigma and must be performed manually by the user in the Figma UI.

Typical examples:
- variant property management
- component property panel actions not exposed through TalkToFigma
- browser-only selection or panel state changes that affect what Figma exposes
- any UI control that cannot be read or changed reliably through automation tools

When blocked by one of these actions:
- describe the exact manual step the user needs to take in Figma
- keep the instruction narrow and concrete
- wait for the user to confirm the change
- re-read the relevant nodes with TalkToFigma to verify the resulting state

## Required Workflow

### 1. Establish Connection

Before changing anything:
1. Join the active Figma channel with `TalkToFigma_join_channel`.
2. Confirm the active design context by reading the relevant node, selection, or document info.

If TalkToFigma is disconnected:
- report that state clearly
- ask the user to reconnect the Figma plugin/socket only when blocked

### 2. Inspect Before Editing

Build evidence from the file before proposing or applying changes:
- identify the exact component set, frame, instance, or text nodes involved
- inspect existing properties and variant axes
- compare repeated elements for drift in size, color, position, or behavior
- verify whether the problem is structural, not just visual

Do not assume a component is well-structured because it looks correct in one variant.

### 3. Model the Design Properly

Prefer a clean component architecture over one-off visual patching.

Use these defaults:
- one property per real behavior axis
- one shared nested component for reused sub-elements
- one canonical text/icon treatment per semantic role
- variant names that reflect product concepts, not temporary visual states

Good examples:
- `Status=active|completed|abandoned`
- `Hover=True|False`
- `With Reminder=True|False`

Avoid:
- duplicate axes that describe the same business concept
- manual per-variant glyph or text drift
- fake examples that should be nested instances

### 4. Edit Minimally

Make the smallest structural change that fixes the problem completely.

Prefer:
- reusing an existing nested component instead of duplicating layers
- fixing the shared source instead of touching every variant individually
- aligning all affected variants only after the source definition is correct

If a desired operation is not directly supported by TalkToFigma, describe the exact manual Figma step needed from the user, then re-read the resulting nodes with TalkToFigma to verify the change.

### 5. Verify Before Claiming Success

Before stating something is fixed:
- re-read the affected nodes or component set
- verify sizes, colors, text, layout, and property usage from the live file
- confirm the fix exists in the shared definition and not only in one example
- call out anything that remains incomplete

Never claim a visual or structural fix based only on intent.

## Design Review Checklist

Run through these when working on components or screens:
- Does every variant differ because of a real property?
- Are repeated sub-elements backed by a shared definition?
- Do labels, icons, and chips use consistent typography and spacing?
- Are states aligned with real product behavior?
- Is the layout anchored correctly, not merely visually centered by accident?
- Can a future editor change the design from one source rather than many copies?

## Figma-Specific Guidance

When cleaning up component sets:
- inspect the parent component set first
- verify which properties are actually wired versus merely declared
- bind properties to the real layers or nested instances they control
- prefer nested instances for reusable chips, badges, icons, or accordion content
- resolve duplicate/conflicting variants by simplifying the axis model

When working on text and icon systems:
- decide the canonical sizes first
- ensure fills match the intended semantic color role
- normalize alignment after size changes
- verify every affected variant, especially those with optional content

When building component examples:
- keep primary semantic content as an editable input field when the component supports it
- show state changes through styling and supporting UI, not by replacing the main title concept with variant-specific copy

When working on interaction states:
- separate persistent business state from transient UI state
- keep hover, expanded, selected, and disabled as explicit UI axes only when needed
- do not encode the same concept in multiple properties

## Variants Per Component

When a component has variants, describe the variant model explicitly before editing or reporting completion.

For each component, capture:
- component name and node ID
- parent property list
- allowed values for each property
- which layers or nested instances each property controls
- default or canonical variant
- any invalid, duplicate, or missing combinations

Use this structure when summarizing a component set:

`Component`: `<name>` (`<node-id>`)

`Properties`:
- `<Property A=value1|value2>`
- `<Property B=True|False>`
- `<Property C=<string>|undefined>`

`Variant behavior`:
- `<Property A>` changes `<behavior>`
- `<Property B>` shows or hides `<layer or instance>`
- `<Property C>` changes `<text, layout, or nested instance>`

`Shared definitions`:
- repeated sub-elements should come from one shared source
- typography and spacing should stay consistent across all variants

`Audit notes`:
- flag unused parent properties
- flag duplicate variants that represent the same state
- flag manual overrides that should be replaced with bindings or nested instances

If the component has many variants, reason from the property matrix rather than from screenshots alone.

## Project Figma Shape

Organize the design file so its structure stays predictable and scalable.

Use two primary horizontal rows:
- a component row
- a page row

### Component Row

The component row is the foundation of the project.

Place here:
- atomic components
- composed components
- component sets with variants
- reusable nested building blocks

Expectations for the component row:
- components are complete before they are relied on by pages
- variants are cleaned up and property-driven
- repeated pieces are shared definitions, not detached copies
- naming is consistent and semantic
- spacing, typography, and color rules are settled here first

### Page Row

The page row is for assembled screens, flows, and high-level layouts.

Build pages only from prepared components in the component row.

Do not design a page by inventing ad hoc UI that the component library does not yet support.
If a page needs a missing element:
- stop page work
- design or refine the missing component first
- then return to assemble the page from the prepared pieces

Expectations for the page row:
- pages are composed from reusable components
- page-specific layout is allowed, but page-specific component invention is not the default
- screens reflect the current component system accurately
- page design serves as composition and validation, not as a substitute for component design

Use the file structure to make progress visible:
- components first
- pages second
- page work only after required components are ready

## Communication

While working:
- keep updates short and factual
- report live state, not assumptions
- mention blockers immediately when connection state prevents verification
- if a manual Figma step is needed from the user, say exactly what action is required and what will be verified afterward

## Boundaries

This skill is for design work in Figma and live design QA.

If the user wants production frontend code, pair or transition to the `frontend-design` or `figma` implementation skills as appropriate.
