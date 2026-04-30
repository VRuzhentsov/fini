# Specs

Feature specs live here as Markdown and are grouped by feature.

These specs are the implementation contract for the main `fini` repo.

## Features

- `device-connect/` - device discovery, pairing, presence, and paired-device lifecycle
- `space-sync/` - pair-scoped space mapping, bootstrap sync, sync sessions, and sync status
- `space/` - local space model and space management behavior

## Convention

- Put cross-cutting domain behavior in `specs/<feature>/README.md`
- Keep view/component companion docs next to the frontend files in `src/**.md`
- In companion docs, link to the feature spec when the behavior belongs to a broader domain concept

## Repo vs Wiki

Keep docs in the main `fini` repo when they are load-bearing for implementation and should change with code reviews.

Put these in `fini/specs`:

- current feature behavior and invariants
- API/runtime contracts
- acceptance criteria that can be tested
- ownership boundaries between views, stores, and backend services

Keep docs in `fini-wiki` when they are broader, historical, strategic, or synthesized across multiple implementation phases.

Put these in `fini-wiki`:

- product rationale and historical intent
- architecture evolution and superseded approaches
- roadmap, planning captures, and decision context
- cross-feature analysis and long-form notes

If both are needed, keep the enforceable contract in `fini/specs` and link to the wiki for rationale/history.

## Current Map

- `specs/device-connect/README.md`
- `specs/space-sync/README.md`
- `specs/space/README.md`
