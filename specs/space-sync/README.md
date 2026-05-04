# Space Sync

## Scope

Per-pair space mapping lifecycle, sync session establishment, bootstrap transfer, sync event replay, and sync status reporting.

## Responsibilities

- Maintain symmetric mapped-space lifecycle state for each paired peer
- Send one-space sync request, acceptance, and end-of-sync events over the authenticated sync channel
- Establish websocket sync sessions for paired peers
- Bootstrap newly mapped spaces
- Replicate sync events and acknowledgements
- Track `last_synced_at` per peer and per mapped space
- Track `end_of_sync_at` when a previously mapped space stops syncing

## Behavior

- Space mapping is pair-scoped and keyed by `space_id`
- Enabling a mapping is a per-space lifecycle request from one device to one paired peer
- Only the receiving device sees the incoming space sync request
- The approval flow is one space at a time; batch snapshot approval is not product behavior
- Already-synced spaces must not trigger approval on startup, reconnect, or normal sync ticks
- Enabling or re-enabling a mapping triggers bootstrap/merge for all quest changes in that space
- Built-in spaces resolve by stable IDs; custom spaces may need approval/resolution to create or map the one incoming space
- Removing a mapping sends an end-of-sync event, writes `end_of_sync_at` on both devices, and stops future sync for that space
- Quest create/update/delete events for active mapped spaces sync silently in the background without additional dialogs
- Sync transport uses authenticated websocket sessions, not discovery metadata
- Visible sync status should converge for both peers after successful bootstrap/event transfer

## Primary UI Surfaces

- `src/views/DeviceView.vue`
- `src/components/DeviceView/IncomingSpaceResolutionDialog.vue`

## Related Features

- `specs/device-connect/README.md`
- `specs/space/README.md`

## Wiki Links

- `~/projects/fini-wiki/pages/concepts/SpaceSync.md` when present
- `~/projects/fini-wiki/pages/concepts/e2e-testing.md` for cross-device QA context
