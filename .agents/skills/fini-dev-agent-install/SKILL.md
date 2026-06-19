---
name: fini-dev-agent-install
description: "Use this to install, repair, or verify local Fini autonomous dev-agent schedules. This Fini repo skill defines the portable contract only; host-specific OpenClaw cron scripts, credentials, locks, and runtime state live in the agent environment."
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
      - name: FINI_REPO
        required: false
        description: Optional GitHub owner/repo override for local agent automation.
      - name: FINI_DAILY_RECIPIENT
        required: false
        description: Optional name to use in the daily report greeting.
      - name: FINI_ISSUE_TOPIC_SYNC_FILE
        required: false
        description: Optional local issue/topic sync file path for dynamic Fini Dev topics.
      - name: FINI_ISSUE_TG_TOPIC_MAP
        required: false
        description: Legacy optional local issue/topic sync file path.
---

# Fini Dev Agent Install

Use this skill to install or repair autonomous Fini dev-agent schedules in a local agent environment. The Fini repo owns the portable behavior contract; concrete scheduler implementation, host crontab wiring, Telegram credentials, GitHub CLI auth, locks, retry state, trusted-author policy, and runtime files belong to the private agent/OpenClaw environment.

Do not add host-specific install scripts to this repository. If a local agent needs executable automation, implement it in that agent's own repo or workspace skill and keep only the expected Fini-facing contract here.

## Expected Jobs

Daily triage report:

- Job ID: `fini-daily-issue-report`
- Schedule: `0 8 * * *`
- Timezone: local timezone, or `FINI_DAILY_TZ` when configured
- Session: isolated
- Prompt: load `fini-daily`, run from the local Fini checkout, use `triage` when available or `fini-daily`'s fallback ranking otherwise, query open GitHub issues and pull requests from `FINI_REPO` or the checkout's `origin` remote, include GitHub URLs, call out stale or near-ready PRs, and send the final report to `FINI_DAILY_TG_TARGET`
- Delivery: Telegram `Daily` topic parsed from `FINI_DAILY_TG_TARGET`

Branch fetch:

- Job ID: `fini-fetch-all-branches`
- Schedule: every `5m`
- Session: isolated, light context
- Prompt: run `git fetch --all --prune` from the local Fini checkout and report only failures
- Delivery: none
- Tool boundary: exec-only or equivalent least-privilege command execution

Merged PR topic reconciliation:

- Optional local agent automation, not a Fini repo implementation.
- May run every `5m` in the private agent environment.
- Should query merged pull requests from `FINI_REPO` or the checkout's `origin` remote.
- Should update only locally ignored issue-topic sync state.
- Should not require source-controlled credentials, host paths, cron IDs, lock paths, or trust policy.

## Issue Topic Sync Contract

Dynamic Fini Dev issue topics may be recorded in a local JSON file. The default local path is `issue-topic-sync.json` at the Fini checkout root unless `FINI_ISSUE_TOPIC_SYNC_FILE` or legacy `FINI_ISSUE_TG_TOPIC_MAP` points elsewhere.

The file is runtime state and must stay untracked. Fini commits may document the shape, but should not commit real topic IDs, chat IDs, tokens, local paths, or author allowlists.

Expected issue entry fields:

- `issue`: GitHub issue number
- `title`: issue title or topic title source
- `issueTarget`: Telegram target in `<group-id>:topic:<thread-id>` form when available
- `topicId`: messenger topic/thread ID when stored separately
- `createdAt`, `mappedAt`, `startedAt`, or `topicCreatedAt`: entry-level mapping timestamp for stale-merge protection
- `pullRequest`: optional related PR URL
- `status`: local lifecycle state such as `open` or `closed`
- `closedAt`: timestamp for local topic closure state
- `closedByPullRequest`: PR URL that caused closure
- `finalTopicNoteStatus`: local notification state such as `pending` or `sent`
- `finalTopicNoteSentAt`: timestamp for final topic note delivery
- `topicTitle`: last reconciled topic title

## Prerequisites

Before installing or repairing local schedules, verify:

1. The local agent runtime is installed and has a writable schedule store.
2. `fini-daily` is installed for the local agent.
3. `triage` is installed for the local agent, or the agent is prepared to use `fini-daily`'s explicit fallback ranking.
4. `FINI_DAILY_TG_TARGET` is set to the Daily topic target in `<group-id>:topic:<thread-id>` form.
5. GitHub access for `FINI_REPO`, or the checkout's inferred GitHub repository, works without printing tokens.
6. Any optional merged-PR topic reconciler has local Telegram credentials and GitHub write access in the private agent environment.

If a required prerequisite is missing, stop and report the exact blocker. Do not create a partial schedule that appears healthy but cannot deliver to the Daily topic. Missing `triage` is not a hard blocker when `fini-daily`'s fallback ranking is available; report that the scheduled job will use fallback ranking until `triage` is installed.

## Prompt Contracts

The daily report prompt must preserve this intent:

```text
Use the fini-daily skill. Run from the local Fini checkout. Use FINI_DAILY_TG_TARGET, FINI_PROGRESS_TG_TARGET, FINI_REPO, and FINI_DAILY_RECIPIENT from the local agent environment when they are set. Query current open GitHub issues and pull requests using configured GitHub access without printing secrets, including the GitHub URL for each item. Run or load triage before choosing the recommendation when triage is available; otherwise use fini-daily's explicit fallback ranking and say fallback ranking was used. Call out stale, blocked, or near-ready pull requests and prefer finishing a stale or close PR over starting a new issue when the ranking supports it. Produce the daily report format with a configured-recipient greeting only when FINI_DAILY_RECIPIENT is set, and with full GitHub links for every listed issue and pull request. Deliver the final report to FINI_DAILY_TG_TARGET.
```

Keep this prompt focused on read-only triage and reporting. Do not edit issues, labels, code, docs, or branches from the daily job unless the user explicitly delegates implementation.

The fetch prompt must preserve this intent:

```text
From the local Fini checkout, run git fetch --all --prune to update every remote branch reference. Do not switch branches, merge, rebase, reset, clean, edit files, or push. Report only if the fetch fails, including the command and error summary.
```

## Idempotency Rules

- Treat `fini-daily-issue-report` and `fini-fetch-all-branches` as the only portable job IDs defined by this repo.
- Do not create timestamped duplicate jobs.
- Do not remove unrelated jobs.
- Do not overwrite a different job unless it has the managed ID.
- Keep private runtime IDs and host crontab markers in the local agent repo or workspace, not in Fini.
- Prefer dry-run output before write output when reporting changes.
- If existing jobs already match, report `changed: false`.

## Verification

Use local agent tooling where available. Expected evidence:

- Exactly one enabled job with ID `fini-daily-issue-report`.
- Exactly one enabled job with ID `fini-fetch-all-branches`.
- The daily job schedule is `0 8 * * *` in the selected timezone.
- The fetch job schedule is every `5m`.
- Delivery points to the Daily topic thread from `FINI_DAILY_TG_TARGET`.
- `fini-daily` is available, and either `triage` is available or fallback ranking is explicitly accepted for the daily job.
- Telegram is configured and can send to the Daily topic, or the blocker is explicitly reported.

If local agent commands are blocked by device-scope approval, verify through any available read-only status path and report the approval blocker. Do not keep retrying approval loops.

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
Optional reconciler: <not installed | installed in local agent environment | blocked>
Evidence: <commands and outcomes>
Blocker: <only if blocked>
```
