---
name: fini-daily
description: "Use for the daily Fini GitHub issue and pull request report, stale PR attention callout, and delegation recommendation. Summarizes open VRuzhentsov/fini issues and PRs for the configured user, recommends the next issue or PR to finish, and uses the configured Fini Dev Telegram targets for daily reports and implementation progress updates."
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
---

# Fini Daily

Use this skill for the daily Fini issue and pull request status report, stale PR attention callouts, and delegation handoff recommendations. This skill is a Fini-specific wrapper around Matt Pocock's `triage` skill.

## Required Triage Step

Before choosing the recommended next issue or PR, execute or load Matt Pocock's `triage` skill and feed it the current open Fini issue and pull request set.

Use `triage` to rank issues and PRs by urgency, impact, user value, unblock potential, age, and fit for an agent-sized implementation session. Treat its output as the priority engine for this report.

If the `triage` skill is unavailable, stop and report that `triage` is missing. Do not invent a replacement ranking silently.

## Operating Context

- Run from the Fini repository root: `~/projects/fini`.
- Repository: `VRuzhentsov/fini`.
- Primary recipient: `<user-name>`.
- Preferred Telegram group: `Fini Dev`.
- Preferred report target comes from `FINI_DAILY_TG_TARGET`.
- Preferred implementation progress target comes from `FINI_PROGRESS_TG_TARGET`.
- Fallback group ID comes from `FINI_DEV_TG_CHANNEL_ID`.

## Inputs

Gather current status from GitHub issues and pull requests before running `triage` and writing the report.

Use the available GitHub access on the Will Claw host. If `gh` needs an explicit token, use the host's configured Fini GitHub token source without printing token values.

Minimum issue fields:

- issue number
- title
- labels
- state
- updated time
- assignee, if any
- milestone, if any
- comments or body summary when needed to understand priority

Minimum pull request fields:

- PR number
- title
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

## Daily Report Format

Write a concise report addressed to `<user-name>`.

Use this structure:

```text
<user-name>, here is the daily Fini issue and pull request report.

Status:
- <short status summary>

Open Issues:
- #<number> <title> — <labels/priority signal> — <current recommendation>

Open PRs:
- PR #<number> <title> — <age/staleness/review/CI signal> — <current recommendation>

Recommended Next Delegation:
- Target: <PR #number or Issue #number> <title>
- Why now: <one sentence>
- Suggested assignment prompt: <copy-pasteable instruction for an autonomous Fini agent>

Risks / Blockers:
- <only meaningful blockers, or "none found">
```

Keep it actionable. Prefer one clear next target over a long menu. If a stale PR should be finished, make that the primary recommendation. Cite that the recommendation came from the `triage` pass.

## Triage Inputs

Pass the following prioritization hints to `triage`:

1. Open non-draft PRs that are stale, close to merge, blocking follow-up work, or just need verification/review fixes.
2. Blockers for current Fini development or release work.
3. Labeled as high priority, MVP, design-required, or current phase work.
4. Small enough for an agent to complete and verify in one focused session.
5. Recently created or updated by the configured user or maintainer.
6. Unassigned and not already in progress.

Ask `triage` to deprioritize issues that are:

- labeled `no-auto` (exclude from autonomous delegation entirely)
- already covered by an open PR unless the PR is abandoned or explicitly blocked
- research-only unless they unblock multiple implementation issues
- vague without enough acceptance criteria
- blocked by missing product/design decisions
- already assigned or clearly in progress

Ask `triage` to deprioritize PRs that are:

- labeled `no-auto`, or linked to an issue labeled `no-auto` (exclude from autonomous delegation entirely)
- draft and still actively being built, unless they have been stale long enough to need explicit attention
- blocked on a user/product decision
- superseded by a newer PR or branch
- failing for reasons that require manual maintainer credentials or release approval

## Delegated Implementation Progress

When the configured user asks an autonomous Fini agent to implement a specific Fini issue or task, report progress to `FINI_PROGRESS_TG_TARGET` when available.

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
