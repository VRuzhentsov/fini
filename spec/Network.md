# Network & Local Sync

Fini is local-first and accountless. LAN sync is introduced in **MVP.1**.

## Goals

- Zero-cloud architecture: no central account service required
- Nearby device discovery on LAN
- Explicit pairing before any data sharing
- Per-space sync selection
- Near-real-time propagation with offline queue/replay

## Entry points

- GUI app
- MCP server
- Headless runtime

All entry points operate on the same local dataset.

## Pairing

- Discovery is passive; data exchange starts only after pairing
- Pairing uses mutual confirmation code (both devices confirm same code)
- Pairing survives restarts until unpaired

## Space selection

- User selects which spaces replicate to each paired device
- Space identity is id-based, not name-based
  - Built-ins use reserved ids: `"1"`, `"2"`, `"3"`
  - Custom spaces use UUID ids

## Sync model

- Push on change + reconnect catch-up
- Offline edits queue locally and replay on reconnect
- Conflict resolution: last-write-wins by `updated_at` UTC
- Deletes replicate globally (tombstone semantics; no resurrection)

## Shared repeating behavior

- Repeating quests use series + occurrences
- One occurrence completion resolves for all paired devices
- Deterministic occurrence identity uses `series_id + period_key`
- `period_key` uses UTC period boundaries
- Completion of shared occurrence cancels pending reminders for that occurrence on all peers

## Focus sync

- Manual and reminder focus override metadata replicate across paired devices
- Main quest is computed from synced data/events, not from a device-local singleton state

## Collaboration metadata

- Completion actor is stored as device hostname (no account identity)
- Teammate completion updates are subtle in-app updates

## Security

- LAN sharing is off by default
- Encrypted transport required for paired communication
- At-rest encryption is post-MVP work
