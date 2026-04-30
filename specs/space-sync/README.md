# Space Sync

## Scope

Per-pair space mapping, sync session establishment, bootstrap transfer, sync event replay, and sync status reporting.

## Responsibilities

- Maintain symmetric mapped-space state for each paired peer
- Send mapping updates to peers over the authenticated sync channel
- Establish websocket sync sessions for paired peers
- Bootstrap newly mapped spaces
- Replicate sync events and acknowledgements
- Track `last_synced_at` per peer and per mapped space

## Behavior

- Space mapping is pair-scoped and keyed by `space_id`
- Enabling a mapping triggers bootstrap sync for that space
- Built-in spaces resolve by stable IDs; custom spaces may need approval/resolution
- Incoming mapping updates surface as an approval flow before applying on the receiving device
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
