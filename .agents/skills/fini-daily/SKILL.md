---
name: fini-daily
description: "Use for the daily Fini GitHub issue and pull request report, stale PR attention callout, and delegation recommendation. Summarizes open Fini issues and PRs, recommends the next issue or PR to finish, and uses the configured Fini Dev Telegram targets for daily reports and implementation progress updates."
metadata:
  openclaw:
    envVars:
      - name: FINI_DEV_TG_CHANNEL_ID
        required: false
        description: Telegram group chat ID for Fini Dev.
      - name: FINI_DAILY_TG_TARGET
        required: false
        description: Preferred Telegram target for daily reports, usually <group-id>:topic:<thread-id>.
      - name: FINI_PROGRESS_TG_TARGET
        required: false
        description: Preferred Telegram target for delegated implementation progress updates.
      - name: FINI_REPO
        required: false
        description: GitHub owner/repo to report on; when unset, infer it from the current Fini checkout.
      - name: FINI_DAILY_RECIPIENT
        required: false
        description: Optional name to use in the daily report greeting.
---

# Fini Daily

Use this skill for the daily Fini issue and pull request status report, stale PR attention callouts, and delegation handoff recommendations.

## Required Triage Step

Before choosing the recommended next issue or PR, execute or load the `triage` skill when it is available and feed it the current open Fini issue and pull request set.

Use `triage` to rank issues and PRs by urgency, impact, user value, unblock potential, age, and fit for an agent-sized implementation session. Treat its output as the priority engine for this report.

If the `triage` skill is unavailable, continue with an explicit fallback ranking instead of stopping:

1. Prefer open, non-draft PRs that are closest to landing, especially those with review feedback, failed checks, merge conflicts, or stale inactivity.
2. Next prefer issues that unblock other Fini work, have clear scope, no `no-auto` label, and fit an agent-sized implementation session.
3. Deprioritize `no-auto`, unclear, design-only, blocked, or broad exploratory work.
4. State in the report that fallback ranking was used because `triage` was unavailable.

## Operating Context

- Run from the current Fini repository root. Do not assume a fixed checkout path; if needed, resolve it with `git rev-parse --show-toplevel` from the loaded repo.
- Repository: `FINI_REPO`, or the GitHub `owner/repo` inferred from the current checkout.
- Primary recipient: `FINI_DAILY_RECIPIENT` when set; otherwise use a neutral report greeting.
- Preferred Telegram group: `Fini Dev`.
- Preferred report target comes from `FINI_DAILY_TG_TARGET`.
- Preferred implementation progress target comes from `FINI_PROGRESS_TG_TARGET`.
- Fallback group ID comes from `FINI_DEV_TG_CHANNEL_ID`.

## Inputs

Gather current status from GitHub issues and pull requests before running `triage` and writing the report.

Use the available GitHub access on the local agent host. If `gh` needs an explicit token, use the host's configured Fini GitHub token source without printing token values.

Minimum issue fields:

- issue number
- title
- GitHub URL (`url` / `html_url`)
- labels
- state
- updated time
- assignee, if any
- milestone, if any
- comments or body summary when needed to understand priority

Minimum pull request fields:

- PR number
- title
- GitHub URL (`url` / `html_url`)
- labels
- draft state
- review decision, if available
- CI/check status, if available
- created time
- updated time
- author
- head branch
- linked issue, if obvious from title/body/branch
- comments or body summary when needed to understand why it is blocked

Treat the GitHub label `no-auto` as a hard automation boundary:

- Include `no-auto` issues in the open-issues report for visibility.
- Do not recommend `no-auto` issues for autonomous implementation or delegation.
- Mark them as manual/design-only or explicitly excluded from auto-pickup.

Use open PRs as first-class daily report inputs:

- A PR that is open and not draft is usually closer to value than starting a new issue.
- A stale PR should be called out even if it is not the final recommendation.
- Treat a PR as stale when it has been open or not updated for several days, especially when it blocks follow-up work, carries user-visible work, or has no clear next owner.
- Prefer recommending "finish PR #<number>" over "start issue #<number>" when the PR is close to merge, has been hanging, or just needs verification/review/fixes.
- Draft PRs should be reported separately as in-progress, not as ready-to-finish unless the next step is clearly to undraft or complete review fixes.

## Project Support Contract

Use this skill as Fini's project-local support layer for daily triage and ticket pickup decisions. Keep the guidance Fini-facing: define what the project expects from issues, pull requests, verification, and review. Do not document private agent internals, credential handling, or host-specific trust mechanics here.

Treat GitHub issues as the authoritative source for auto-starting implementation work. When daily triage recommends a `ready-to-start` issue for delegated implementation, the expected loop is:

1. start from the issue body, labels, linked context, and current comments
2. create or reuse an issue-numbered branch
3. implement and verify the smallest useful slice
4. for implementation changes, wait for the user-verification gate required by `AGENTS.md` before committing, pushing, or opening the PR
5. for docs, specs, process guidance, or implementation work after required user verification, push the branch
6. create or update the linked pull request as the review artifact

The daily recommendation should make this PR expectation explicit in the suggested assignment prompt. A delegated issue is not fully handed off when only a branch exists; it should have a PR URL, or a stated blocker explaining why the PR could not be created.

Classify candidate issues with these readiness states:

- `ready-to-start`: the issue has a clear goal, bounded scope, expected behavior, and a practical verification path. It may be recommended for delegated implementation when it has no exclusion label and no stronger PR should be finished first.
- `needs-clarification`: the issue is potentially useful, but scope, product intent, expected behavior, or verification is ambiguous. Recommend one concise clarification question instead of implementation.
- `needs-human-review`: the issue touches security, privacy, data migration, release behavior, broad architecture, user-visible product direction, or another high-risk area. Include it for visibility, but do not recommend autonomous pickup without explicit maintainer direction.

Treat these Fini signals as project-specific triage inputs:

- `no-auto` always excludes an issue or linked PR from autonomous pickup.
- Stale or near-ready PRs usually outrank starting a fresh issue.
- Issues that lack acceptance criteria, reproduction steps for bugs, or verification expectations should be marked `needs-clarification`.
- Tickets involving release, credentials, privacy, destructive data behavior, database migrations, or external service behavior should be marked `needs-human-review` unless the issue explicitly narrows the work to documentation or dry-run planning.
- Prefer a small, verifiable implementation slice over broad exploratory work.

When recommending a next delegation, include the readiness state and the reason it is safe or unsafe to start. If no issue is ready, recommend the highest-value clarification or review action instead of forcing a pickup.

## Daily Report Format

Write a concise report for the configured recipient. If `FINI_DAILY_RECIPIENT` is unset, omit the personal salutation.

Use this structure:

```text
<recipient>, here is the daily Fini issue and pull request report.

Status:
- <short status summary>

Open Issues:
- #<number> <title> - <GitHub URL> - <labels/priority signal> - <current recommendation>

Open PRs:
- PR #<number> <title> - <GitHub URL> - <age/staleness/review/CI signal> - <current recommendation>

Recommended Next Delegation:
- Target: <PR #number or Issue #number> <title> - <GitHub URL>
- Why now: <one sentence>
- Suggested assignment prompt: <copy-pasteable instruction for an autonomous Fini agent>

Risks / Blockers:
- <only meaningful blockers, or "none found">
```

Keep it actionable. Prefer one clear next target over a long menu. If a stale PR should be finished, make that the primary recommendation. Cite whether the recommendation came from the `triage` pass or the explicit fallback ranking.

Every listed issue and pull request must include its full GitHub URL in the report. Do not rely on bare `#<number>` or `PR #<number>` references because Telegram will not reliably resolve them to GitHub.

## Triage Inputs

Pass the following prioritization hints to `triage`, or apply them directly when using fallback ranking:

1. Open non-draft PRs that are stale, close to merge, blocking follow-up work, or just need verification/review fixes.
2. Blockers for current Fini development or release work.
3. Labeled as high priority, MVP, design-required, or current phase work.
4. Small enough for an agent to complete and verify in one focused session.
5. Recently created or updated by the configured recipient or maintainer.
6. Unassigned and not already in progress.

Deprioritize issues that are:

- labeled `no-auto` (exclude from autonomous delegation entirely)
- already covered by an open PR unless the PR is abandoned or explicitly blocked
- research-only unless they unblock multiple implementation issues
- vague without enough acceptance criteria
- blocked by missing product/design decisions
- already assigned or clearly in progress

Deprioritize PRs that are:

- labeled `no-auto`, or linked to an issue labeled `no-auto` (exclude from autonomous delegation entirely)
- draft and still actively being built, unless they have been stale long enough to need explicit attention
- blocked on a user/product decision
- superseded by a newer PR or branch
- failing for reasons that require manual maintainer credentials or release approval

## Delegated Implementation Progress

When the configured user asks an autonomous Fini agent to implement a specific Fini issue or task, create or reuse the issue/task-specific Telegram topic and keep detailed progress there.

Use `FINI_PROGRESS_TG_TARGET` only as a fallback for generic implementation status or when topic creation/reuse is unavailable.

Progress updates should include:

- issue/task identifier
- current phase: investigating, implementing, verifying, blocked, PR ready
- branch name, when created
- evidence collected or verification command planned
- blocker, if any

Do not spam. Send progress updates at meaningful phase transitions, not after every small command.

## Boundaries

- The daily report is read-only: do not edit issues, labels, code, or docs unless the user explicitly delegates implementation or maintenance.
- If implementing a delegated issue, follow `fini-dev` first and obey the repo workflow.
- Do not merge PRs or push directly to `main`.
- Do not expose tokens or secret file contents.
- If Telegram delivery is unavailable, return the report text and state the delivery blocker.

## Verification

Before reporting success, include evidence:

- GitHub issue query succeeded, with count of open issues reviewed.
- GitHub PR query succeeded, with count of open PRs reviewed.
- Delivery target used, or delivery blocker.
- If a next target is recommended, cite the concrete PR/issue labels, age, review/CI, and update signals that drove the recommendation.
