---
name: fini-dev-agent
description: "Use this when an autonomous-coding-agent, autonomous coding agent, remote agent, Will Claw/Milo, or other delegated agent is asked to implement, debug, verify, or report progress on Fini repository work. This is a behavior overlay for agents: load `fini-dev` for Fini development mechanics, then use this skill to organize scope, autonomy, Telegram topic coordination, progress updates, blockers, verification evidence, and handoff discipline."
metadata:
  openclaw:
    envVars:
      - name: FINI_DEV_TG_CHANNEL_ID
        required: false
        description: Telegram group chat ID for Fini Dev.
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

## Telegram Topic Coordination

Use Fini Dev Telegram topics as coordination surfaces when the channel is available. Do not let Telegram delivery failures block local implementation; continue the work and report the delivery blocker in the final handoff.

Preferred targets:

| Topic | Use For | Preferred Env Var |
|---|---|---|
| `Daily` | Daily issue reports, triage summaries, next-delegation recommendations | `FINI_DAILY_TG_TARGET` |
| `Create` | New ticket intake, issue drafting, scope capture, task creation status | `FINI_CREATE_TG_TARGET` |
| `In Progress` | Implementation progress, blockers, verification updates, PR-ready notices | `FINI_PROGRESS_TG_TARGET` |

Fallback:

- Use `FINI_DEV_TG_CHANNEL_ID` only when a topic-specific target is not configured.
- Prefer topic targets in the `<group-id>:topic:<thread-id>` form when available.
- Never print or expose bot tokens, GitHub tokens, or secret file contents.

Progress messages should be short and phase-based:

```text
Fini #<issue-or-task>: <phase>
- Working on: <one sentence>
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
- verification commands and outcomes
- manual checks still needed
- whether Telegram progress delivery succeeded or failed

## Non-Goals

This skill does not define Fini product semantics, test commands, release rules, design bundle protocol, Android workflow, or CLI behavior. Load the specialized Fini skill for those areas through `fini-dev`.
