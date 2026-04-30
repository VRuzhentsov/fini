# Device Connect

## Scope

Device discovery, add-device mode, pairing handshake, paired-device persistence, and online presence.

## Responsibilities

- Discover nearby devices that are eligible for pairing
- Enter and leave add-device mode
- Drive sender/receiver pairing flow with a 6-digit passcode
- Persist paired devices locally
- Track online/offline presence for paired peers
- Provide peer endpoint metadata needed by `space-sync`

## Behavior

- Only devices in add-device mode are pairing candidates
- Pairing uses a sender/receiver handshake with a 6-digit passcode
- Sender sees the code only after receiver acceptance
- Pairing completion persists both peers as paired devices
- Presence is refreshed independently from pairing state
- Discovery metadata is untrusted and only used to find candidate peers/endpoints

## Primary UI Surfaces

- `src/views/SettingsView.vue`
- `src/views/AddDeviceView.vue`
- `src/views/DeviceView.vue`

## Related Feature

- `specs/space-sync/README.md`

## Wiki Links

- `~/projects/fini-wiki/pages/concepts/DeviceConnection.md` when present
- `~/projects/fini-wiki/pages/concepts/device-sync-architecture.md` for architecture history
