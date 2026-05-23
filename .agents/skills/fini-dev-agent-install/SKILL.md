---
name: fini-dev-agent-install
description: "Use this to install, repair, or verify the Fini autonomous dev agent schedules. Trigger when asked to set up fini-dev-agent, local autonomous agent scheduling, daily 8 AM triage, every-5-minutes branch fetch, Daily topic reports, OpenClaw cron for Fini, or rerunnable/idempotent install of the Fini dev agent. This skill ensures a stable daily 8 AM local-time job runs `fini-daily`, uses `triage`, reports to the Fini Dev `Daily` Telegram topic, and keeps all Git branches fetched every 5 minutes."
metadata:
  openclaw:
    envVars:
      - name: FINI_DEV_TG_CHANNEL_ID
        required: false
        description: Telegram group chat ID for Fini Dev.
      - name: FINI_DAILY_TG_TARGET
        required: true
        description: Preferred Telegram target for Daily topic reports, usually <group-id>:topic:<thread-id>.
      - name: FINI_PROGRESS_TG_TARGET
        required: false
        description: Preferred Telegram target for delegated implementation progress updates.
      - name: FINI_DAILY_TZ
        required: false
        description: Optional timezone override for the 8 AM daily schedule.
---

# Fini Dev Agent Install

Use this skill to install or repair autonomous Fini dev-agent schedules. The workflow is intentionally idempotent: repeated runs should converge on the same managed cron jobs without duplicating schedules or changing unrelated jobs.

## Outcome

Ensure the local agent has these schedules:

Daily triage report:

- Job ID: `fini-daily-issue-report`
- Schedule: `0 8 * * *`
- Timezone: local timezone, or `FINI_DAILY_TZ` when configured
- Session: isolated
- Prompt: load `fini-daily`, run from `~/projects/fini`, use `triage`, query open `VRuzhentsov/fini` GitHub issues, and send the final report to `FINI_DAILY_TG_TARGET`
- Delivery: Telegram `Daily` topic parsed from `FINI_DAILY_TG_TARGET`

Branch fetch:

- Job ID: `fini-fetch-all-branches`
- Schedule: every `5m`
- Session: isolated, light context
- Prompt: run `git fetch --all --prune` from `~/projects/fini` and report only failures
- Delivery: none

## Prerequisites

Before writing schedule state, verify:

1. OpenClaw is installed and the local gateway or cron store path is available.
2. `fini-daily` is installed for the local agent.
3. `triage` is installed for the local agent.
4. `FINI_DAILY_TG_TARGET` is set to the Daily topic target in `<group-id>:topic:<thread-id>` form.
5. GitHub access for `VRuzhentsov/fini` works without printing tokens.

If a prerequisite is missing, stop and report the exact blocker. Do not create a partial schedule that cannot deliver to `Daily`.

## Helper Script

Use the bundled helper to upsert the cron job safely:

```bash
node .agents/skills/fini-dev-agent-install/scripts/upsert-fini-daily-cron.mjs --dry-run
node .agents/skills/fini-dev-agent-install/scripts/upsert-fini-daily-cron.mjs --write
```

The helper:

- Reads `~/.openclaw/cron/jobs.json` by default.
- Preserves unrelated cron jobs.
- Replaces only the managed jobs with IDs `fini-daily-issue-report` and `fini-fetch-all-branches`.
- Defaults to dry-run and requires `--write` to modify the store.
- Uses `FINI_DAILY_TG_TARGET` for Telegram delivery.
- Uses `FINI_DAILY_TZ` or the local system timezone for the 8 AM schedule.

Use `--store <path>` only for tests or non-standard OpenClaw stores.

## Verification

After `--write`, verify with the local agent tools when available:

```bash
openclaw cron list --json
openclaw skills info fini-daily
openclaw skills info triage
openclaw channels status --probe
```

Expected evidence:

- Exactly one enabled job with ID `fini-daily-issue-report`.
- Exactly one enabled job with ID `fini-fetch-all-branches`.
- The job schedule is `0 8 * * *` in the selected timezone.
- The fetch job schedule is every `5m`.
- Delivery points to the `Daily` topic thread from `FINI_DAILY_TG_TARGET`.
- `fini-daily` and `triage` are available.
- Telegram is configured and can send to the Daily topic, or the blocker is explicitly reported.

If OpenClaw CLI cron commands are blocked by device-scope approval, verify by reading the cron list through any available read-only status path and report the approval blocker. Do not keep retrying approval loops.

## Prompt Contracts

The scheduled prompt must preserve this intent:

```text
Use the fini-daily skill. Run from ~/projects/fini. Use FINI_DAILY_TG_TARGET and FINI_PROGRESS_TG_TARGET from the local agent environment. Query current open GitHub issues for VRuzhentsov/fini using configured GitHub access without printing secrets. Run or load triage before choosing the recommendation. Produce the daily report format addressed to Vitalii. Deliver the final report to FINI_DAILY_TG_TARGET.
```

Keep this prompt focused on read-only triage and reporting. Do not edit issues, labels, code, docs, or branches from the daily job unless the user explicitly delegates implementation.

The fetch prompt must preserve this intent:

```text
From ~/projects/fini, run git fetch --all --prune to update every remote branch reference. Do not switch branches, merge, rebase, reset, clean, edit files, or push. Report only if the fetch fails, including the command and error summary.
```

## Idempotency Rules

- Treat `fini-daily-issue-report` and `fini-fetch-all-branches` as the only managed cron jobs.
- Do not create timestamped duplicate daily jobs.
- Do not remove unrelated jobs.
- Do not overwrite a different job unless it has the managed ID.
- Prefer dry-run output before write output when reporting changes.
- If the existing job already matches, report `changed: false`.

## Report Format

When finished, report:

```text
Status: <installed | already current | blocked>
Job: fini-daily-issue-report
Schedule: 0 8 * * * @ <timezone>
Delivery: <group-id>:topic:<thread-id>
Job: fini-fetch-all-branches
Schedule: every 5m
Delivery: none
Evidence: <commands and outcomes>
Blocker: <only if blocked>
```
