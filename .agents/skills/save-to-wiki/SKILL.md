---
name: save-to-wiki
description: "Use this when the user asks to save discussed information, plans, decisions, research, summaries, or context to the project wiki raw folder for future ingestion. Trigger on phrases like 'save this to wiki', 'write this plan to raw', 'save this discussion for later ingestion', 'put this in the wiki raw folder', or when the user wants durable wiki source material without immediate ingestion."
---

# Save To Wiki Raw

Save the current discussion, plan, decision, research summary, or other user-approved context as a new raw source document in the sibling project wiki.

This skill is for durable capture only. It creates raw source material for future postponed ingestion; it does not synthesize wiki pages under `pages/`.

## Target Location

Resolve the wiki path from the current repository name:

```text
current repo: <repo-name>
wiki root: ../<repo-name>-wiki/
raw folder: ../<repo-name>-wiki/raw/
```

Examples:

```text
repo: fini
wiki raw: ../fini-wiki/raw/
```

If the sibling wiki or `raw/` folder does not exist, stop and report the expected path. Do not create a new wiki structure unless the user explicitly asks.

## Preflight

Before writing:

1. Identify the current workspace basename as the repo name.
2. Resolve `../<repo-name>-wiki/AGENTS.md` and read it if present.
3. Resolve `../<repo-name>-wiki/raw/`.
4. Choose a dated, kebab-case filename.
5. Check that the filename does not already exist.
6. If it exists, append `-2`, `-3`, etc.

## Filename Convention

Use:

```text
YYYY-MM-DD-<short-kebab-title>.md
```

Good examples:

```text
2026-04-26-mdns-sd-device-discovery-architecture.md
2026-04-26-reusable-synced-devices-e2e-precondition.md
2026-04-26-release-dry-run-notes.md
```

Prefer a concise descriptive title over a generic one like `notes.md`.

## Raw Document Structure

Use this structure by default, trimming sections that clearly do not apply:

```markdown
# <Title>

Date: YYYY-MM-DD

## Context

Why this was discussed or captured.

## Summary

The short version.

## Decisions

Locked decisions or agreed direction.

## Plan

Implementation or follow-up plan, if any.

## Evidence

Concrete evidence, commands, links, outputs, code references, or sources that support the claims.

## Open Questions

Unresolved decisions, risks, or follow-ups.
```

For transcript-like saves, preserve the user's intent and important corrections. Do not invent decisions that were not made.

## Rules

- Create a new file under `raw/` only.
- Treat existing `raw/` files as immutable; do not edit them.
- Do not update `_hot.md`, `_index.md`, `log.md`, or `pages/**` unless the user explicitly asks for ingestion.
- Preserve exact commands, paths, issue numbers, and decision wording when they matter.
- Mark uncertainty explicitly in `Open Questions` instead of asserting guesses.
- Keep secrets and credentials out of the raw doc.
- If the discussion references sensitive local files such as `.env`, summarize without copying sensitive content.

## Reporting

After writing, report:

- created path
- one-line summary
- whether ingestion was intentionally deferred

Example:

```text
Saved to `../fini-wiki/raw/2026-04-26-mdns-sd-device-discovery-architecture.md`.
Summary: locks mdns-sd as the discovery provider and WebSocket as the pairing/sync path.
Ingestion deferred; no wiki pages were updated.
```
