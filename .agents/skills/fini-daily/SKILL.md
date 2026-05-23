---
name: fini-daily
description: "Use for the daily Fini GitHub issue report and delegation recommendation. Summarizes open VRuzhentsov/fini issues for Vitalii, recommends the next issue to solve, and uses the configured Fini Dev Telegram targets for daily reports and implementation progress updates."
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

Use this skill for the daily Fini issue status report and for delegation handoff recommendations. This skill is a Fini-specific wrapper around Matt Pocock's `triage` skill.

## Required Triage Step

Before choosing the recommended next issue, execute or load Matt Pocock's `triage` skill and feed it the current open Fini issue set.

Use `triage` to rank issues by urgency, impact, user value, unblock potential, and fit for an agent-sized implementation session. Treat its output as the priority engine for this report.

If the `triage` skill is unavailable, stop and report that `triage` is missing. Do not invent a replacement ranking silently.

## Operating Context

- Run from the Fini repository root: `~/projects/fini`.
- Repository: `VRuzhentsov/fini`.
- Primary recipient: Vitalii.
- Preferred Telegram group: `Fini Dev`.
- Preferred report target comes from `FINI_DAILY_TG_TARGET`.
- Preferred implementation progress target comes from `FINI_PROGRESS_TG_TARGET`.
- Fallback group ID comes from `FINI_DEV_TG_CHANNEL_ID`.

## Inputs

Gather current status from GitHub issues before running `triage` and writing the report.

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

Treat the GitHub label `no-auto` as a hard automation boundary:

- Include `no-auto` issues in the open-issues report for visibility.
- Do not recommend `no-auto` issues for autonomous implementation or delegation.
- Mark them as manual/design-only or explicitly excluded from auto-pickup.

## Daily Report Format

Write a concise report addressed to Vitalii.

Use this structure:

```text
Vitalii, here is the daily Fini issue report.

Status:
- <short status summary>

Open Issues:
- #<number> <title> — <labels/priority signal> — <current recommendation>

Recommended Next Delegation:
- Issue: #<number> <title>
- Why now: <one sentence>
- Suggested assignment prompt: <copy-pasteable instruction for Will Claw/Milo>

Risks / Blockers:
- <only meaningful blockers, or "none found">
```

Keep it actionable. Prefer one clear next issue over a long menu. Cite that the recommendation came from the `triage` pass.

## Triage Inputs

Pass the following prioritization hints to `triage`:

1. Blockers for current Fini development or release work.
2. Labeled as high priority, MVP, design-required, or current phase work.
3. Small enough for an agent to complete and verify in one focused session.
4. Recently created or updated by Vitalii.
5. Unassigned and not already in progress.

Ask `triage` to deprioritize issues that are:

- labeled `no-auto` (exclude from autonomous delegation entirely)
- research-only unless they unblock multiple implementation issues
- vague without enough acceptance criteria
- blocked by missing product/design decisions
- already assigned or clearly in progress

## Delegated Implementation Progress

When Vitalii asks Will Claw/Milo to implement a specific Fini issue or task, report progress to `FINI_PROGRESS_TG_TARGET` when available.

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
- Delivery target used, or delivery blocker.
- If a next issue is recommended, cite the concrete labels/update signals that drove the recommendation.
