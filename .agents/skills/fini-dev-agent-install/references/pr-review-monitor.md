# Optional PR Review Monitor Contract

Use this reference when a Fini agent needs a reusable contract for monitoring pull request feedback. This document describes behavior that can be shared across agents without committing one developer's runtime cron state, credentials, Telegram topics, or public-reply trust policy.

## Scope

The monitor watches open Fini pull requests for actionable feedback from trusted authors:

- unresolved review threads
- new review comments
- PR issue comments
- review decision changes
- failed checks or mergeability blockers

It may evaluate, fix, reply to, and resolve feedback only when the local agent policy allows the public action.

## Keep Local

Do not commit these to the Fini repository:

- concrete cron job IDs or scheduler storage
- queue, state, or lock files
- Telegram chat IDs, topic IDs, bot tokens, or gateway-specific routes
- GitHub tokens or authenticated user details
- the active trusted-author whitelist for public replies
- local paths outside the checkout, except `~`-anchored examples in docs

The repository may document schemas and behavior, but each agent owns its own runtime configuration.

## Suggested Local Files

Agents can choose their own paths, but the files should be explicit and separate:

- whitelist config, such as `~/.fini/pr-review-monitor/whitelist.json`
- queue, such as `~/.fini/pr-review-monitor/queue.json`
- state, such as `~/.fini/pr-review-monitor/state.json`
- lock directory, such as `~/.fini/pr-review-monitor/lock`
- Fini issue/topic sync file, usually `issue-topic-sync.json` at the checkout root or `FINI_ISSUE_TOPIC_SYNC_FILE`

Example whitelist shape:

```json
{
  "githubAuthors": [
    "VRuzhentsov",
    "chatgpt-codex-connector",
    "chatgpt-codex-connector[bot]"
  ]
}
```

## Queue Contract

The monitor should acquire an atomic lock before reading or writing queue/state. If the lock is fresh, exit quietly. If the lock is stale, reacquire it and record that recovery happened.

Use deterministic queue keys so repeated scans do not duplicate work:

- `pr:<number>:review-thread:<threadId>`
- `pr:<number>:review-comment:<id>`
- `pr:<number>:issue-comment:<id>`
- `pr:<number>:check:<checkName>:<conclusion>:<startedAt>`

Queue statuses:

- `queued`: discovered and ready for a worker
- `in_progress`: a worker has started and recorded current progress
- `waiting`: blocked on time, data, another worker, or user decision
- `blocked`: cannot continue without external action
- `done`: completed, with result and public URLs when applicable
- `skipped`: intentionally not processed, with reason

Rules:

- Process the existing queue before scanning GitHub for new items.
- Process only a small bounded number of items per run.
- Do not enqueue duplicates already in `queued`, `in_progress`, `waiting`, or `blocked`.
- Treat fresh `in_progress` items as active; move stale ones to `waiting` unless there is evidence of completion.
- A start or dispatch message is not enough. Each item must show progress, a worker, a blocker, or an outcome.
- `done` items must include a concise result and, when applicable, GitHub reply or resolution URLs.

## Evaluation Gate

Before any public reply or review-thread resolution, classify the feedback:

- `valid`: the comment identifies a real defect, missing behavior, unclear contract, or meaningful risk.
- `already addressed`: the requested change is already present in the current branch.
- `not applicable`: the comment is outside scope, based on a false premise, or does not apply to the current code path.
- `needs decision`: correctness depends on product intent, risk acceptance, user preference, or external policy.

Include one or two evidence bullets grounded in code, docs, test output, GitHub state, or runtime state.

If the evaluation is `needs decision`, or if evidence is uncertain, do not post a confident GitHub reply. Mark the queue item `waiting` or `blocked` and ask in the mapped task topic.

## Topic Routing

Progress belongs in the mapped PR or issue topic:

1. Resolve PR topic mapping from `pullRequests[<pr>].target` or `topicId`.
2. Otherwise resolve an issue mapping whose `pullRequest` or `closedByPullRequest` matches the PR URL.
3. If no topic is mapped, record `missing_topic_mapping` in state and stay silent.

Never fall back to a root or General topic for PR-specific work.

The same topic should contain:

- task start
- investigation notes
- evaluation
- implementation progress
- verification evidence
- public reply URL, or a reason no public reply was posted
- final outcome

## Public Reply Policy

Public GitHub replies are external actions. A local agent may auto-reply only when all of these are true:

- the author is trusted by the local whitelist
- the reply is factual, low-risk, and scoped to the current PR
- the fix or evidence has been verified
- the reply does not make a product decision, accept risk, expose secrets, or depend on private unrelated context

For untrusted authors or risky replies, summarize the evaluation in the mapped topic and ask before posting publicly.

## GitHub Query Discipline

- List open PRs with a bounded query first.
- Process one PR at a time.
- Prefer small REST or `gh` calls.
- Use GraphQL per PR only when review-thread state is needed, with bounded `reviewThreads(first:50)` and `comments(first:10)`.
- Existing unresolved review threads are actionable by current state, even if they predate the monitor or topic mapping.
- For old backlog, enqueue unresolved `review-thread` tasks only; do not fan out every historical review or comment.

## Verification

After installing or changing a local monitor, verify:

- scheduler entry exists and is enabled
- lock is released after a run
- queue JSON parses
- state has no configuration error
- a mapped PR can receive start, evaluation, progress, and outcome messages in its own topic
- public replies, if posted, include GitHub URLs and correspond to resolved or completed queue items
