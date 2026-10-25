# Specs

Feature specs live here as Markdown and are grouped by feature.

These specs are the implementation contract for the main `fini` repo.

## Features

- `space/` - local space model and space management behavior
- `backup/` - portable zip import/export for quests and spaces

## Convention

- Put cross-cutting domain behavior in `specs/<feature>/README.md`
- Keep view/component companion docs next to the frontend files in `src/**.md`
- In companion docs, link to the feature spec when the behavior belongs to a broader domain concept

## What Belongs Here

Keep docs in the `fini` repo when they are load-bearing for implementation and should change with code reviews:

- current feature behavior and invariants
- API/runtime contracts
- acceptance criteria that can be tested
- ownership boundaries between views, stores, and backend services

## Current Map

- `specs/space/README.md`
- `specs/backup/README.md`

Pairing, channels and space sync are in `docs/architecture/communication.md`.
Specs move into `docs/` when they change (see the `fini-documentation` skill).
