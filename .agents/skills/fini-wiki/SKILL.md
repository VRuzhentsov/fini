---
name: fini-wiki
description: "Use this for Fini wiki work: querying existing project knowledge from the sibling fini-wiki repo, or saving durable raw source material there. Trigger when the user asks what the wiki says, needs product/domain/terminology/historical/architecture context, asks about prior decisions or current project semantics, or asks to save/document/capture/write plans, decisions, research, summaries, or context into the wiki raw folder. For Fini questions that may depend on historical product context, query the wiki even if the user does not explicitly say 'wiki'."
---

# Fini Wiki

Query existing Fini project knowledge or save durable raw source material in the sibling project wiki.

The wiki is the project's second brain. Use it when the task needs product semantics, historical decisions, current architecture context, terminology, or durable capture for future ingestion.

## Mode Selection

Choose one mode based on the user's intent:

- Query mode: the user asks a question about Fini context, product/domain meaning, current or historical architecture decisions, terminology, prior plans, or what the wiki says.
- Raw capture mode: the user asks to save, document, capture, write down, or put discussed information into the wiki for future ingestion.

If both apply, query first to ground the answer, then ask before saving new raw source material unless the user already explicitly requested capture.

## Query Mode

Answer questions against the sibling wiki before relying on memory.

### Query Target

Resolve the wiki path from the current repository name:

```text
current repo: <repo-name>
wiki root: ../<repo-name>-wiki/
```

Examples:

```text
repo: fini
wiki root: ../fini-wiki/
```

If the sibling wiki does not exist, report the expected path and answer from repo evidence only if available.

### Query Protocol

1. Read `../<repo-name>-wiki/AGENTS.md` if present and follow its schema.
2. Read `../<repo-name>-wiki/_hot.md` first for active context and high-signal facts.
3. Read `../<repo-name>-wiki/_index.md` second to locate relevant pages.
4. Read 1-2 targeted pages under `pages/` when the right pages are obvious.
5. Use targeted search under `pages/**/*.md` when `_hot.md` and `_index.md` do not reveal the right page.
6. Stay within five wiki pages unless the user explicitly asks for deeper research.

### Query Answers

- Cite concrete wiki files or wikilinks for wiki-derived claims.
- If implementation behavior matters, combine wiki context with targeted source/spec reads from the main repo.
- Say explicitly when the wiki lacks evidence for a claim.
- For substantive new synthesis, offer to save it as raw wiki material, but do not save automatically unless requested.

Good query triggers include:

- "What does the wiki say about SpaceSync?"
- "Why did we choose this sync architecture?"
- "Can you tell me about the modal dialog situation?"
- "Is this behavior consistent with the historical product direction?"
- "What's the current terminology for device pairing?"

## Raw Capture Mode

Save the current discussion, plan, decision, research summary, or other user-approved context as a new raw source document in the sibling project wiki.

Raw capture creates source material for future postponed ingestion; it does not synthesize wiki pages under `pages/`.

Full wiki ingestion into `_hot.md`, `_index.md`, `log.md`, or `pages/**` is intentionally out of scope unless the user explicitly asks for ingestion.

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
