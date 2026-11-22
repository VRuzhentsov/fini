---
name: dev
description: "Always use this at the start of development work in the Fini repo. Gives the dev process (Spike and Stabilize) and orchestrates which repo-local or gstack skill to load, which Makefile command to prefer, what context to read, and what evidence is required before claiming implementation, debugging, QA, Android, design-to-code, release, or documentation work is complete."
---

# Fini development

The development pattern is **Spike and Stabilize** (Dan North). Spike:
implement the requested feature first, with no tests, then verify it manually
against the requirements. Stabilize: only then refactor and bring the tests up
to date. Follow the steps below in order. `AGENTS.md` holds the project
instructions; `docs/development.md` holds the commands and the development
rules.

Write in STE from the first draft ("Use STE" below), and verify with concrete
evidence before you report success.

## 1. Understand the change

1. Run the first-checkout install bootstrap once before normal routing.
2. If the work is driven by a GitHub issue, create or switch to a branch whose name includes the issue number before editing, such as `issue-123-short-topic` or `fix/123-short-topic`. Reuse an existing branch if it already contains the issue number, and pause before switching branches if the worktree has conflicting uncommitted changes.
3. Inspect the ticket labels. When a Fini ticket has the `design` label, load `fini-design` before planning, updating, closing, or implementing it.
4. State the target outcome, in-scope work, out-of-scope work, and success checks in the user's terms.
5. Read only the context needed for the task: `AGENTS.md`, `docs/development.md`, `docs/naming.md` and `docs/glossary.md` (for each task that writes code or text), the relevant page in `docs/architecture/`, the relevant folder `README.md` and companion `.md` specs where they still exist, `docs/adr/`, and targeted source files.
6. Choose the scoped skill from "Skill routing" below.
7. If the user explicitly asks to think, plan, brainstorm, or review without implementation, do that mode first and do not edit files until execution is requested.

## 2. Decide the design

1. For any planning stage, load `grill-me` before finalizing the plan: explicit think/plan/brainstorm requests, ticket scoping, architecture plans, design plans, release plans, QA strategy, and any pre-implementation decision work. Use it as an overlay on the domain skill. Gather evidence from the repo, specs, ADRs, or source files first; ask the user only for decisions evidence cannot resolve, one question at a time, with the recommended answer. Stop when the remaining questions are non-blocking or explicitly deferred.
2. Shape the change as `docs/architecture/code-design.md` says: find the object that owns the data and the interface it implements, search for code that does the job already, and put the change there.
3. Apply "No legacy in alpha" in `docs/development.md`.
4. When the user must choose, follow "Asking the user for a decision" in `AGENTS.md`.
5. Pick the manual checks against the requirements before editing (step 4), so the success standard is clear.

## 3. Spike: write the code

1. Before you write or review code, read `docs/code-style.md`.
2. Start the HMR processes and iterate on them (`docs/development.md`, "Development runs on HMR; the user builds artefacts"). Prefer the Makefile targets in "Make targets and what CI runs"; do not invent targets.
3. Implement the requested feature. Do not write tests yet.
4. Establish evidence for the current behavior first: code path, spec, failing output, test result, log, or reproducible step. Trace write path, persisted data, and read path when data behavior is involved.
5. Make the smallest scoped change that addresses the root cause or requested behavior. Avoid broad refactors and compatibility layers unless they prevent a concrete defect. Putting a behavior on the existing interface or object that owns it is not a broad refactor: do it. Remove the code that your change makes unused.
6. Preserve user or other-agent changes in the worktree.
7. Keep paths and instructions portable (`docs/development.md`, "Portable files").
8. Keep the PR a draft while the work is in progress. CI skips drafts (`.github/workflows/ci.yml`).
9. Prove each step with a compile check (`cargo check`, `npm run build`) and manual checks through the devtools bridge (`docs/android.md`, "The devtools bridge"): the happy path and the basic edge cases of that step. Commit and push each step that passes.
10. Leave test files alone during the draft. A test that fails after a planned behaviour change stays failing until step 5; do not edit its expectation to match the new code, and do not run or fix the suites per step — the next step can change the behaviour again. The only allowed edit is a compile adaptation (a changed function signature), named in the report.

## 4. Spike: verify manually

1. Check the feature against the requirements on the running apps. Seed and query state through `fini-cli` (`docs/development.md`, "Dev data"); use the UI for visual confirmation only. Use `fini` instead when the dev task maps directly to an end-user action (e.g. "create a quest titled X due Friday in Family"). Drive the apps as `docs/testing.md`, "Drive the running apps" says.
2. Keep a list of the checks and their results; step 5 turns it into test proposals. Continue only when the feature works as the requirements say.
3. Gate to leave draft: prove the feature's happy flow end to end on the running apps or real hardware, and put the evidence in the PR description: what was run, on which devices, and the result. For Bluetooth, that means pairing both ways, the session going green, and a quest syncing both ways. Then mark the PR ready for review; that starts CI.

Verification for each area:

- Frontend-only logic or UI: run the narrowest available type/build check (`npm run build`); use browser/QA skill when visual behavior matters.
- Backend Rust or Tauri command changes: run the narrowest Rust or app build check available; include command output evidence. After a Rust change, confirm that the running app has it.
- Cross-process, sync, or persistence changes: verify write path, storage/outbox/database effects, and read path.
- E2E-sensitive flows: in the stabilize phase, run the affected e2e projects (`fini-test`, "Stabilize"). Use `make e2e-headed` for local visible debugging. For CI-parity evidence, run what CI runs: `npm run gate:e2e:build-image` and `npm run gate:e2e:run`.
- Android behavior: load `fini-android-testing`; the deploy targets and when to use each are in `docs/android.md`.
- Fini CLI or app binary behavior: load `fini-cli`; use `make runtime-smoke` for runtime container CLI checks or `make build` for a local release-profile binary. Use the CLI for all programmatic interaction with the app; do not use webview or IPC MCP tooling to drive the app.
- Runtime/container behavior beyond the CLI surface: use `make runtime-smoke` or the relevant image target.
- Release work: load `fini-release`. Releases run only in CI, through the Release Button that the user starts. Do not create or push tags.

If a command cannot be run, say why, what evidence is missing, and the exact command the user can run.

## 5. Stabilize

1. Once the system works as designed, refactor. Review the diff against the "Review checklist" in `docs/code-style.md`, and the scoped skill's checklist.
2. Bring the tests up to date: load `fini-test` and do its "Stabilize" steps. For each failing test, report what it expects and which decision changed that behaviour, then fix, clean up or delete it as the user decides.
3. Show the user the test cases you propose to add, based on the manual checks from step 4, and add the ones they choose.
4. Do the STE check of the full branch diff ("Use STE", step 7).
5. Get CI green, and bring the PR title and description up to date with everything the branch now contains.

## 6. Update docs and hand off

1. Update the docs that describe the changed behavior or structure, and `docs/glossary.md` for new terms. Load `fini-documentation` for each doc change.
2. Commit and push to the active PR branch (`AGENTS.md`, "Workflow"). The user reviews and merges. One worker keeps one branch and one PR for all of its work (`git-workflow`).
3. Report, concisely:
   - What changed, with file references.
   - Verification evidence, with commands and outcomes.
   - The STE check: that you read the diff for STE, and each new term that needs the approval of the user.
   - Any limits, skipped checks, or residual risks.

Lead with evidence when the user explicitly asks for proof.

## Skill routing

Load the scoped skill when its condition applies. When no skill fits, continue with this skill's process.

| Condition | Skill |
|---|---|
| Create, update, list, or manage quests, spaces, reminders, or Focus state through the Fini CLI | `fini`, which uses `fini-cli` |
| Use, validate, or reason about the Fini app binary, CLI mode, app launch mode, or runtime container CLI behavior | `fini-cli` |
| Populate data, seed state, or exercise a feature against the running/installed app for dev or testing | `fini-cli` |
| Validate Android behavior, prove Android navigation/state, or debug Android-only behavior | `fini-android-testing` |
| Run, write, debug, or organize unit, integration, or e2e tests (step 5). For Android on a physical phone, use `fini-android-testing` instead | `fini-test` |
| Ticket has GitHub label `design`, or work designs/refines native Figma components, variants, screens, visual systems, or Fini UI surfaces | `fini-design` |
| Change Vue frontend code under `src/`, especially view components, templates, conditional rendering, lists, or frontend tests | `fini-frontend`; also load `fini-test` for test authoring/execution |
| Change SQLite schema, Diesel schema, data migrations, migration rollback, or database migration tests | `fini-dev-db`; also load `fini-test` for migration coverage |
| Add or change Makefile targets, npm scripts, `xtask`, CI command orchestration, build tooling, packaging tooling, or repo-local automation architecture | `fini-scripting`; before you edit `package.json`, read `docs/code-style.md`, "package.json scripts" |
| Make a release, bump a release version for shipping, push a release tag, inspect release readiness, fix release automation, or verify release CI | `fini-release`; also load `fini-versioning` for metadata semantics and `fini-scripting` when automation changes are needed |
| Change package metadata, app version display, CLI version output, Android versioning, or CI release version sync outside an operational release | `fini-versioning`; also follow `fini-scripting` when automation changes are needed |
| Prepare a major release, Play Store listing assets, Android marketplace screenshots, or release screenshot packages | `fini-release-prep`; also follow `fini-scripting` when automation changes are needed |
| Debug errors, regressions, stack traces, crashes, or unexpected behavior | `investigate` |
| QA a web/app flow and fix bugs found | `qa` |
| QA report only, without fixes | `qa-only` |
| Browser dogfooding, screenshots, forms, responsive checks, or live site interaction | `browse` or `gstack` |
| Frontend UI construction or visual polish in code | `fini-frontend`, then `frontend-design` when visual design-system work is needed |
| Code review or pre-landing diff review | `review` |
| Ship, push, create PR, or prepare code for landing | `ship` |
| Merge, deploy, or production verification | `land-and-deploy` or `canary` |
| Performance, page speed, bundle size, or regression checks | `benchmark` |
| Security audit, threat model, OWASP, secrets, or supply-chain concerns | `cso` |
| Create or improve skills | `skill-creator` |
| Planning, scoping, or decision-tree clarification before implementation | `grill-me`, plus the relevant domain skill |
| Create, start, or draft tickets | `fini-create-ticket`, `start-ticket`, or `ticket-markdown` |
| Add, move, or change docs, specs, READMEs, ADRs, the glossary, or architecture pages | `fini-documentation` |
| Work as an autonomous, remote, or delegated agent | `fini-dev-agent` |
| Work in the Fini Dev Telegram group, or route work across its topics | `fini-dev-telegram` |
| Write the daily Fini issue and pull request report | `fini-daily` |
| Install, repair, or verify local Fini dev-agent schedules | `fini-dev-agent-install` |

## Use STE

Fini writes in ASD-STE100 Simplified Technical English (STE): names in code, comments, log messages, docs, screen text, commit messages, PR text, agent instructions, and what you write to the user. Do the STE check yourself; the user does not ask for it.

1. Read `docs/naming.md` before you write any of these.
2. For each new name, comment and log message, do the check in `docs/naming.md`, "Check a name before you write it". Look up each word in the STE dictionary.
3. Use a technical noun or verb only when it is a term in `docs/glossary.md`.
4. When no approved word or glossary term gives the meaning, tell the user the new term: the term, its one meaning, and why no approved word fits. Add it to the glossary only after you get the approval of the user.
5. Before you make a new file, tell the user its name and its job. Make it only after you get approval.
6. Before each commit, read your staged diff for STE: each new or changed name, comment, log message, assertion message, test title, doc sentence, and the commit message. Look up each word that is not clearly approved in the text copy of the dictionary (`docs/naming.md`, "Get the standard"). Replace the words in the glossary's list "Words deliberately not used".
7. Before you push the last commit of a PR stage, and before you report a PR as ready, read the full branch diff (`git diff origin/main...HEAD`) for STE in the same way, and fix what fails. Do this each time, with no request from the user.
8. Keep the names of the language, of libraries and of tools as they are (`spawn`, `setup-java`, `LinuxBackend`). A Fini name that is not STE changes, also when it comes from older code.
9. Talk to the user in STE too. Tell the meaning of each term before you use it.

During the change to STE, change the names, comments, and text of each file that you change. Leave other files as they are.
