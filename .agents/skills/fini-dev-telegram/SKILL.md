---
name: fini-dev-telegram
description: "Use when participating in the Fini Dev Telegram group, routing Fini development work across predefined topics, creating issue topics, or discussing Fini agent/process improvements."
metadata:
  openclaw:
    envVars:
      - name: FINI_DEV_TG_CHANNEL_ID
        required: false
        description: Telegram group chat ID for Fini Dev.
      - name: FINI_ISSUE_TG_TOPIC_MAP
        required: false
        description: Optional JSON file mapping Fini GitHub issue numbers to Telegram topic targets.
      - name: FINI_DAILY_TG_TARGET
        required: false
        description: Preferred Telegram target for Daily topic reports, usually <group-id>:topic:<thread-id>.
      - name: FINI_CREATE_TG_TARGET
        required: false
        description: Preferred Telegram target for Create topic work intake and ticket creation updates.
      - name: FINI_PROGRESS_TG_TARGET
        required: false
        description: Preferred Telegram target for In Progress topic implementation updates.
---

# Fini Dev Telegram

Use this skill for Fini-specific behavior inside the `Fini Dev` Telegram group. It defines where work belongs, when to create topic threads, and how an agent should participate without turning every message into a status update.

This skill is about the agent and process around developing Fini, not the Fini product itself. Product feature ideas should become GitHub issues or issue-specific topics.

## Topic Map

Default Fini Dev topics:

- `General`: loose coordination, questions that do not yet belong elsewhere, and fallback delivery.
- `Daily`: daily reports, triage summaries, next-delegation recommendations, and scheduler-level summaries.
- `Create`: new issue intake, ticket drafting, scope capture, and "turn this idea into work" requests.
- `In Progress`: generic implementation progress when no issue-specific topic exists.
- `Fini Self Improvement`: improving the Fini development agent, skills, topic routing, reports, schedules, handoff format, or Telegram working model.
- Dynamic issue topic `#<issue> <short title>`: all detailed progress, blockers, evidence, PR-ready handoff, and closure notes for one active GitHub issue.

Use topic targets in `<group-id>:topic:<thread-id>` form when available. Fall back to `FINI_DEV_TG_CHANNEL_ID` only when no topic target is configured.

## Routing Rules

- If the message asks for a Fini product change, route it to `Create` or an existing dynamic issue topic.
- If the message asks how the agent should behave while developing Fini, keep it in `Fini Self Improvement`.
- If the message is a daily/status question across issues and PRs, route it to `Daily`.
- If the message is implementation progress for one GitHub issue, route it to that issue's dynamic topic.
- If the message is implementation progress without a GitHub issue yet, use `In Progress` until an issue exists.
- If the user is brainstorming process but not asking for implementation, respond with triage and selectable action items before editing files.

## Dynamic Issue Topics

Every GitHub issue that an autonomous Fini agent is actively working on should have its own Fini Dev Telegram forum topic.

Before sending issue-specific progress:

1. Check the topic map at `FINI_ISSUE_TG_TOPIC_MAP` or, if unset, `fini-issue-topics.json` at the local Fini checkout root.
2. If the issue already has an `issueTarget`, use it for all progress, blockers, verification evidence, and PR-ready handoff.
3. If no mapping exists and Telegram topic creation is available, create a topic named `#<issue> <short title>`.
4. Record the issue number, title, GitHub URL, topic id, and `<group-id>:topic:<thread-id>` target immediately.
5. Send a short starting message inside the new issue topic so future readers know the branch, PR, and phase.

Use `In Progress` only for generic status, scheduler work, or work not tied to one GitHub issue. Do not put detailed implementation updates for a specific issue in `Daily` or the root Fini Dev topic once its dynamic issue topic exists.

## Closing Issue Topics

When a pull request for a mapped issue is merged:

1. Close the related GitHub issue if GitHub did not close it automatically.
2. Rename the Telegram forum topic so the title begins `closed #<issue>`.
3. Update the topic map entry with `status: "closed"`, `closedAt`, `closedByPullRequest`, and `topicTitle`.
4. Send one short final note inside the issue topic with the merged PR URL and issue close status.

The `fini-merged-pr-topic-reconcile` system cron may perform this idempotently. Agents may also do it immediately during handoff after a merge, preserving the same map fields and title convention.

## Message Discipline

Send Telegram messages only at meaningful boundaries:

- accepted / starting
- investigating
- implementing
- verifying
- blocked
- PR ready / handoff ready
- closed / merged

Progress messages should be short:

```text
Fini #<issue-or-task>: <phase>
- Working on: <one sentence>
- Evidence: <file, command, log, or current finding>
- Next: <next concrete step>
- Blocker: <only if blocked>
```

Do not send progress after every command or minor edit. In group chats, answer when directly asked or when adding concrete value.

## Daily Report Links

Daily Fini reports posted to Telegram must include full GitHub URLs for every listed issue and pull request. Do not rely on bare `#<number>` or `PR #<number>` references because Telegram will not reliably resolve them to GitHub.

## Boundaries

- Do not expose bot tokens, GitHub tokens, secret file contents, or private personal context.
- Do not act as Vitalii's voice in the group.
- Do not merge PRs or push to `main` from Telegram coordination alone.
- Do not convert `Fini Self Improvement` discussions into code changes unless Vitalii selects or confirms an action item.
