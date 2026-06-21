---
name: fini-dev-agent
description: "Use this when an autonomous coding agent, remote agent, or other delegated agent is asked to implement, debug, verify, or report progress on Fini repository work. This is a behavior overlay for agents: load `fini-dev` for Fini development mechanics, then use this skill to organize scope, autonomy, Telegram topic coordination, progress updates, blockers, verification evidence, and handoff discipline. If work arrives from the Telegram `Create` topic, always route it through `fini-create-ticket` as ticket intake before treating it as implementation work."
metadata:
  openclaw:
    envVars:
      - name: FINI_DEV_TG_CHANNEL_ID
        required: false
        description: Telegram group chat ID for Fini Dev.
      - name: FINI_ISSUE_TOPIC_SYNC_FILE
        required: false
        description: Optional JSON file syncing Fini GitHub issue numbers to messenger topic/thread targets.
      - name: FINI_ISSUE_TG_TOPIC_MAP
        required: false
        description: Legacy optional JSON file mapping Fini GitHub issue numbers to Telegram topic targets.
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

# Fini Autonomous Dev Agent

Use this skill as a behavior overlay for autonomous coding agents working on Fini. It does not replace `fini-dev`; it adds delegation, progress, and handoff discipline around it.

When the work is happening in the Fini Dev Telegram group, also load `fini-dev-telegram` for group-topic routing, Fini Self Improvement handling, and issue-topic coordination.

## First Step

Load `fini-dev` before changing Fini code, tests, docs, automation, release files, or designs.

Then apply this skill for autonomous-agent behavior:

- Keep the scope locked to the delegated issue, ticket, prompt, or user request.
- Work end-to-end within that scope when feasible.
- Preserve user and other-agent changes in the worktree.
- Send progress only at meaningful phase transitions.
- Report verifiable evidence before claiming success.

## Agent Operating Contract

Start each delegated task by establishing:

- target outcome in the user's words
- in-scope files, behavior, or issue
- out-of-scope work that should not be touched
- success checks and smallest useful verification command
- expected progress channel, if Telegram is available

If the task comes from a GitHub issue, follow `fini-dev` branch guidance before editing. If the issue is ambiguous, inspect the issue body, labels, linked design/spec/wiki context, and source code before asking the user.

## Issue Readiness And Review Contract

Treat the configured tracker ticket as the authoritative work source for delegated Fini implementation. The current Fini instance uses GitHub issues and pull requests, but tracker providers and communication channels belong to the agent environment. Before implementation, classify the delegated ticket in Fini-facing terms:

- `ready-to-start`: the ticket has a clear goal, bounded scope, expected behavior, and a practical verification path.
- `needs-clarification`: the ticket is actionable only after a missing scope, product, behavior, or verification decision is answered.
- `needs-human-review`: the ticket touches security, privacy, data migration, release behavior, broad architecture, user-visible product direction, or another high-risk area that should not proceed without explicit maintainer direction.

Treat the Fini `no-auto` label as a hard exclusion from autonomous pickup. Include `no-auto` tickets in reports and handoffs for visibility, but do not classify them as `ready-to-start` or begin branch, commit, push, or review-artifact work unless the maintainer explicitly overrides that ticket's exclusion for the current task.

Do not expose private agent internals, credential handling, channel routing, or host-specific trust mechanics in tracker tickets, review artifacts, or messenger progress. Report only the Fini-facing contract: scope, assumptions, evidence, risk, and review needs.

When a ticket is `needs-clarification`, ask the smallest useful question and include the recommended answer. When it is `needs-human-review`, explain the Fini project risk and stop before implementation unless the maintainer explicitly delegates the next step.

When a ticket is `ready-to-start` and not excluded by labels or maintainer direction, the agent loop should move from authoritative ticket source to branch to review artifact without waiting for a separate "create PR" prompt:

1. Create or reuse a ticket-numbered branch before editing when the tracker has stable ticket numbers.
2. Implement the smallest verified slice described by the ticket.
3. Run the smallest useful local verification and report the evidence.
4. For implementation changes, wait for the user-verification gate required by `AGENTS.md` before committing, pushing, or opening the PR.
5. For docs, specs, process guidance, or implementation work after required user verification, push the branch to the configured remote.
6. Create or update the linked review artifact, such as a pull request for a GitHub-backed project.
7. Report the review URL, readiness state, verification evidence, and remaining risks in the configured progress channel or handoff.

If a host or policy prevents creating a public review artifact automatically, stop at the pushed branch and report that the missing review artifact is a delivery blocker, not a completed handoff.

For agent-assisted pull requests, include a review payload in the PR body or handoff:

- linked ticket
- readiness state used at start
- assumptions made
- files or Fini areas changed
- verification commands and outcomes
- remaining risks
- whether human review is required before merge

## Telegram Topic Coordination

Use Fini Dev Telegram topics as coordination surfaces when the channel is available. Do not let Telegram delivery failures block local implementation; continue the work and report the delivery blocker in the final handoff.

### Conversation Context

In the Fini Dev Telegram group, treat messages in the same forum topic as shared task context even when the newest message does not explicitly mention the bot. The group may be configured to ingest Fini Dev messages without requiring an explicit bot mention.

When invoked from Fini Dev:

1. Read the current message, reply chain, and recent same-topic context before deciding what the user wants.
2. If the newest message is a follow-up, interpret it against the nearest replied-to message and the recent topic conversation.
3. Continue the active task when the follow-up is consistent with the current thread, instead of asking for a fresh explicit command.
4. Stay quiet when the message is ordinary human discussion and no useful action or correction is needed.
5. Never expose private workspace, credential, or unrelated personal context into the group while using prior messages.

### Dynamic Issue Topics

Every GitHub issue that an autonomous Fini agent is actively working on should have its own Fini Dev Telegram forum topic.

Before sending progress for issue work:

1. Check the issue/topic sync file at `FINI_ISSUE_TOPIC_SYNC_FILE`, then legacy `FINI_ISSUE_TG_TOPIC_MAP`, or, if both are unset, `issue-topic-sync.json` at the local Fini checkout root.
2. If the issue already has an `issueTarget`, use that for all issue-specific progress, blockers, verification evidence, and PR-ready handoff.
3. If no mapping exists and Telegram topic creation is available, create a topic named `#<issue> <short title>` in the Fini Dev group.
4. Record the mapping immediately with the issue number, title, GitHub URL, topic id, `<group-id>:topic:<thread-id>` target, and `createdAt` timestamp.
5. Send a short starting message inside the new issue topic so future readers know the branch, PR, and current phase.

Use the shared `In Progress` topic only for generic status, scheduler work, or work that is not tied to one GitHub issue. Do not put detailed implementation updates for a specific issue in `Daily` or the root Fini Dev topic once its dynamic issue topic exists.

### Closing Issue Topics

When a pull request for a mapped issue is merged, the issue-specific Telegram topic should be marked closed:

1. Close the related GitHub issue if GitHub did not close it automatically.
2. Rename the Telegram forum topic so the title begins `closed #<issue>`.
3. Send one short final note inside the issue topic with the merged PR URL and issue close status.
4. Update the topic map entry with `status: "closed"`, `closedAt`, `closedByPullRequest`, `topicTitle`, `finalTopicNoteStatus: "sent"`, and `finalTopicNoteSentAt`.

If the optional `fini-merged-pr-topic-reconcile` local automation is installed, it may perform this idempotently on its own schedule. When that automation is unavailable or unknown, agents should perform the closure steps during merge handoff and preserve the same map fields and title convention.

### Create Topic Intake

When the task source, channel, or thread is the Telegram `Create` topic, load `fini-create-ticket` after `fini-dev` and treat the work as ticket intake, issue drafting, scope capture, or follow-up creation. Do not treat `Create` topic messages as implementation delegation until a ticket or explicit implementation scope exists. Send ticket-creation status updates to `FINI_CREATE_TG_TARGET` when configured.

### Per-Task Topic Binding

Every active Fini implementation, debugging, verification, release, or skill task should have a specific Telegram topic when topic creation is available.

At task start:

1. Reuse the current Telegram topic when the user delegated the work from a task-specific topic.
2. For GitHub issue work, create or reuse a topic named `#<issue> <short title>`.
3. For direct PR or no-issue work, create or reuse a short task topic named after the deliverable, such as `Fini install skill`.
4. Send the first progress message inside that topic before editing when possible.
5. Keep later progress, blockers, verification, PR links, and handoff in the same topic.

The starting message must include the local checkout context so the developer can see where the agent is working:

- worktree path, from `pwd` or `git rev-parse --show-toplevel`
- branch, from `git branch --show-current`
- HEAD, from `git rev-parse --short HEAD`
- worktree state, from `git status --short --branch`
- related issue, branch, or PR when known

If Telegram topic creation or delivery fails, continue locally, then report the failed topic action in the handoff with the exact branch and worktree state.

Preferred targets:

| Topic | Use For | Preferred Target Source |
|---|---|---|
| `Daily` | Daily issue reports, triage summaries, next-delegation recommendations | `FINI_DAILY_TG_TARGET` |
| `Create` | New ticket intake, issue drafting, scope capture, task creation status | `FINI_CREATE_TG_TARGET` |
| `In Progress` | Implementation progress, blockers, verification updates, PR-ready notices | `FINI_PROGRESS_TG_TARGET` |
| Dynamic issue topic `#<issue> <title>` | All progress for one active GitHub issue | `issueTarget` read from the issue entry in `FINI_ISSUE_TOPIC_SYNC_FILE` or legacy `FINI_ISSUE_TG_TOPIC_MAP` |

Fallback:

- Use `FINI_DEV_TG_CHANNEL_ID` only when a topic-specific target is not configured.
- Prefer topic targets in the `<group-id>:topic:<thread-id>` form when available.
- Never print or expose bot tokens, GitHub tokens, or secret file contents.

Progress messages should be short and phase-based:

```text
Fini #<issue-or-task>: <phase>
- Working on: <one sentence>
- Worktree: <path>
- Branch: <branch> @ <short-sha>
- Status: <clean | short status summary>
- Evidence: <file, command, log, or current finding>
- Next: <next concrete step>
- Blocker: <only if blocked>
```

Send progress when entering these phases:

- accepted / starting
- investigating
- implementing
- verifying
- blocked
- PR ready / handoff ready

Do not send progress after every command or minor edit.

## Autonomous Work Loop

Use this loop after `fini-dev` routing chooses the domain skill:

1. Build the evidence chain before editing: trigger path, current behavior, relevant spec/wiki/source, and expected behavior.
2. Create or update a concise todo list for tasks with three or more meaningful steps.
3. Read only the files needed for the delegated scope.
4. Search for existing utilities, constants, commands, and patterns before adding new ones.
5. Make the smallest correct change.
6. Update companion specs/docs when behavior or architecture changes.
7. Verify with the smallest useful command, escalating only when the result demands it.
8. Summarize evidence, changed files, skipped checks, and remaining risks.

When data behavior is involved, verify all three hops before claiming success:

- write path
- persisted data or durable side effect
- read/render/report path

## Decision Discipline

Prefer action over asking when evidence can answer the question. Ask the user only when a decision changes product intent, scope, data semantics, release policy, or risk acceptance.

When blocked:

- State the exact blocker.
- Include the evidence that proves the blocker.
- Offer the shortest safe manual step if user action is faster than automation.
- Keep unrelated work untouched.

Do not expand scope to opportunistic refactors, cleanup, dependency upgrades, or broad test rewrites unless they are required for the delegated task.

## Worktree Safety

Assume the Fini worktree may contain user or other-agent changes.

- Inspect status before edits when implementation is requested.
- Do not revert, overwrite, or restage unrelated changes.
- If existing changes are in files you need to edit, read them carefully and layer your change around them.
- If unrelated changes conflict directly with the delegated task, stop and ask for the desired resolution.
- Do not commit implementation changes until the user has verified they work, following the repo `AGENTS.md` rule.

## Handoff Format

Use this concise final or Telegram handoff format:

```text
Status: <done | blocked | partial>
Scope: <issue/task>
Changed: <files or none>
Evidence: <commands/logs/code refs that verify the claim>
Skipped: <checks not run and why>
Risks: <remaining risks or none>
Next: <one concrete next step, if any>
```

For PR-ready work, include:

- branch name
- issue number or task source
- readiness state used at start
- assumptions made
- verification commands and outcomes
- remaining risks
- whether human review is required before merge
- manual checks still needed
- whether configured progress delivery succeeded or failed

## Non-Goals

This skill does not define Fini product semantics, test commands, release rules, design bundle protocol, Android workflow, or CLI behavior. Load the specialized Fini skill for those areas through `fini-dev`.
