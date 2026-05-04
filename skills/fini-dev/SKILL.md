---
name: fini-dev
description: "Always use this at the start of development work in the Fini repo. Orchestrates which repo-local or gstack skill to load, which Makefile command to prefer, what context to read, and what evidence is required before claiming implementation, debugging, QA, Android, design-to-code, release, or documentation work is complete."
---

# Fini Development Orchestrator

Use this skill as the first step for development work in this repository. It is a routing and evidence checklist, not a replacement for specialized skills.

## Outcome

Keep every development session consistent:

- Pick the right specialized skill before doing domain work.
- Use the repo's documented context path instead of guessing.
- Prefer Makefile targets over raw commands.
- Make the smallest correct change.
- Verify with concrete evidence before reporting success.

## Start Of Work

Before implementation, debugging, QA, Android, design-to-code, release, or documentation changes:

1. State the target outcome, in-scope work, out-of-scope work, and success checks in the user's terms.
2. Read only the context needed for the task: `AGENTS.md`, the relevant folder `README.md`, companion `.md` specs, and targeted source files.
3. If the task needs product, business, terminology, strategic, architecture background, or historical context, load `fini-wiki` and follow the wiki query protocol.
4. Choose any specialized skill from the routing table below before doing the domain-specific work.
5. Pick the smallest useful verification command before editing so the success standard is clear.

If the user explicitly asks to think, plan, brainstorm, or review without implementation, do that mode first and do not edit files until execution is requested.

## Planning Capture

At the end of any major planning session, load `fini-wiki` and write the result to the project wiki raw folder for future ingestion.

Treat a planning session as major when it produces durable context, such as:

- architecture or implementation plans spanning multiple files or systems
- roadmap, scope, or product decisions
- design, UX, or workflow plans
- release, deploy, or QA strategy
- debugging or investigation conclusions that explain root cause and next steps

Do not save trivial one-step plans, routine command plans, or temporary scratch reasoning.

When saving, include:

- original user goal
- final plan
- decisions made
- evidence reviewed, including key files, commands, logs, or docs
- open questions
- explicitly deferred work

Use `fini-wiki` as raw durable capture only. Do not update `_hot.md`, `_index.md`, `log.md`, or `pages/**` unless the user explicitly asks for ingestion.

## Skill Routing

Load the specialized skill when its condition applies:

| Condition | Skill |
|---|---|
| Create, update, list, or manage quests, spaces, reminders, or Focus state through the Fini CLI | `fini`, which uses `fini-cli` |
| Use, validate, or reason about the Fini app binary, CLI mode, app launch mode, or runtime container CLI behavior | `fini-cli` |
| Validate Android behavior, prove Android navigation/state, or debug Android-only behavior | `android-testing` |
| Design or refine native Figma components, variants, screens, or visual systems | `fini-design` |
| Add or change Makefile targets, npm scripts, `xtask`, CI command orchestration, build tooling, packaging tooling, or repo-local automation architecture | `fini-scripting` |
| Change package metadata, app version display, CLI version output, Android versioning, release commands, signed tags, or CI release version sync | `fini-versioning`; also follow `fini-scripting` when automation changes are needed |
| Query product/domain/history/architecture context from the wiki, or save plans, decisions, research, or conversation context to wiki raw material | `fini-wiki` |
| Debug errors, regressions, stack traces, crashes, or unexpected behavior | `investigate` |
| QA a web/app flow and fix bugs found | `qa` |
| QA report only, without fixes | `qa-only` |
| Browser dogfooding, screenshots, forms, responsive checks, or live site interaction | `browse` or `gstack` |
| Frontend UI construction or visual polish in code | `frontend-design`, then preserve existing Fini patterns |
| Code review or pre-landing diff review | `review` |
| Ship, push, create PR, or prepare code for landing | `ship` |
| Merge, deploy, or production verification | `land-and-deploy` or `canary` |
| Performance, page speed, bundle size, or regression checks | `benchmark` |
| Security audit, threat model, OWASP, secrets, or supply-chain concerns | `cso` |
| Create or improve skills | `skill-creator` |
| Create, start, or draft tickets | `create-ticket`, `start-ticket`, or `ticket-markdown` |

When no specialized skill fits, continue with this skill's project workflow.

## Project Context Rules

Use the repo structure as the default map:

- Frontend: `src/`, Vue 3, TypeScript, Vite, Tailwind CSS, DaisyUI, Pinia.
- Backend: `src-tauri/`, Rust, Tauri 2, Diesel, SQLite.
- Domain specs and companion specs: `spec/`, folder `README.md` files, and sidecar `.md` files next to source files.
- Repo automation: `Makefile` is the primary human execution entrypoint, `npm run` owns JS/TS package tasks, and `xtask/` owns non-trivial automation logic. See `fini-scripting`.

Before changing a significant source file, read its companion `.md` spec when present. Write code to match the spec, or update docs/specs deliberately when the behavior changes.

For product semantics, read wiki context using this order:

1. `~/projects/fini-wiki/_hot.md`
2. `~/projects/fini-wiki/_index.md` if needed
3. One or two targeted wiki pages
4. Targeted search only if the right page is not obvious

Stay within the five-page wiki limit unless the user asks for deeper research.

## Command Selection

Prefer these Makefile targets over raw `npm`, `tauri`, or container commands:

| Need | Command |
|---|---|
| Desktop dev app | `make dev` |
| Release desktop build | `make build` |
| Visible local two-app E2E | `make e2e` or `make e2e-headed` |
| Containerized CI-style E2E | `make e2e-ci` |
| Build/update E2E images | `make e2e-image` or `make e2e-actors-image` |
| Full containerized actor E2E | `make e2e-actors` |
| Runtime container image | `make runtime-image` |
| Runtime CLI smoke | `make runtime-smoke` |
| Android device list | `make android-devices` |
| Android Wi-Fi connect | `make android-connect` |
| Android hot reload | `make android-dev` |
| Android build | `make android-build` |
| Android debug deploy | `make android-debug-deploy` |
| Android local release deploy | `make android-release-deploy-local` |
| Release commit + signed tag | `make release VERSION=x.y.z` |

Use package scripts directly only when no Makefile target exists or when a narrower check is clearly better, such as `npm run build` for frontend type/build validation.

Use `fini-scripting` for command architecture details. Default to Makefile for human entrypoints, `npm run` for JS/TS package scripts, and `xtask/` for non-trivial repo automation logic.

Do not invent generic targets such as `make test`, `make lint`, or `make check` unless they exist in the current `Makefile`. If a desired target is missing, name the closest existing target from the table or state that no Makefile target exists for that check.

## Development Loop

Use this loop for implementation and fixes:

1. Establish evidence for the current behavior: code path, spec, failing output, test result, log, or reproducible step.
2. Trace write path, persisted data, and read path when data behavior is involved.
3. Make the smallest scoped change that addresses the root cause or requested behavior.
4. Avoid broad refactors, compatibility layers, or new abstractions unless they prevent a concrete defect.
5. Preserve user or other-agent changes in the worktree.
6. Update companion docs/specs when behavior or structure changes.
7. Verify with the smallest useful check, then escalate only as needed.
8. Report the exact evidence collected and any remaining risk.

## Verification Defaults

Choose verification based on touched area:

- Frontend-only logic or UI: run the narrowest available unit/type/build check; use browser/QA skill when visual behavior matters.
- Backend Rust or Tauri command changes: run the narrowest Rust or app build check available; include command output evidence.
- Cross-process, sync, or persistence changes: verify write path, storage/outbox/database effects, and read path.
- E2E-sensitive flows: use `make e2e-headed` for local visible debugging or `make e2e-ci` for CI parity.
- Android behavior: load `android-testing`; use `make android-devices`, `make android-connect`, and `make android-dev` for dev-runtime verification, or `make android-debug-deploy` when an installed APK check is needed.
- Fini CLI or app binary behavior: load `fini-cli`; use `make runtime-smoke` for runtime container CLI checks or `make build` for release binary creation.
- Runtime/container behavior beyond the CLI surface: use `make runtime-smoke` or the relevant image target.
- Release work: follow release tag rules in `AGENTS.md`; do not create or push tags unless explicitly requested.

If a command cannot be run, say why, what evidence is missing, and the exact command the user can run.

## Reporting Pattern

Final responses for development work should include:

- What changed, with file references.
- Verification evidence, with commands and outcomes.
- Any limits, skipped checks, or residual risks.

Keep this concise. Lead with evidence when the user explicitly asks for proof.
