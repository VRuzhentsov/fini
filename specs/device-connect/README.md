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
- Maintain independent per-pair Network and Bluetooth transport state

## Behavior

- Only devices in add-device mode are pairing candidates
- The user chooses the channel (Network or Bluetooth) **before** discovery runs; it is never inferred from whichever radio found the peer first — see `docs/adr/0008-a-channel-is-a-thing-the-user-chose.md`
- Pairing uses a sender/receiver handshake with a 6-digit passcode
- Sender sees the code only after receiver acceptance
- Trust is established once per pair, not per channel: adding a second channel to an already-paired device requires no passcode and does not interrupt the peer
- Pairing completion persists both peers as paired devices
- Presence is refreshed independently from pairing state
- Discovery metadata is untrusted and only used to find candidate peers/endpoints
- Network and Bluetooth are independent transport providers behind a shared, transport-neutral peer protocol (`PeerFrame`); both stay connected to a paired peer simultaneously, each with its own gray/amber/green liveness proven continuously via bidirectional app-level ping/ack — see `docs/adr/0001-transport-neutral-peer-protocol.md` and `docs/adr/0003-transport-liveness-unified-status-and-manual-switching.md`'s revision
- Exactly one of the currently-connected transports is "primary" (carries real application traffic) at a time: Network whenever it's connected, unless the pair is manually pinned to Bluetooth — the pin only decides which connected transport is primary, not whether the other one dials/stays connected at all
- Each channel has its own per-pair switch: `bluetooth_enabled` (default off — opt in) and `network_enabled` (default on — every existing pair is already syncing over it). Turning a channel off stops its dial loop, closes its session, and releases a pin naming it
- Switching a channel on never fails and never reverts: if the condition it needs is absent the channel stays on, reports `BluetoothAdapterOff` ("on, waiting"), and starts by itself once the condition clears
- Bluetooth enablement stores only the peer Bluetooth address and the local verification time after the user action succeeds; the stored address is diagnostic metadata and is not what a dial connects to
- Fini app pairing is the trust boundary for pairing/control/sync messages. OS Bluetooth pairing is not a precondition for the Bluetooth transport and is not checked: a peer is found by scanning for Fini's service UUID and identified by the app-level `Auth` exchange — see `docs/adr/0006-bluetooth-without-an-os-bond.md`
- Bluetooth discovery/connection metadata is untrusted until the existing Fini pair-auth session succeeds
- Disabling or unpairing a device prevents future Bluetooth use for that Fini pair and clears stored Bluetooth reconnect metadata
- Local device identity is stored as scalar settings rows: `device.id` for immutable UUID and `device.name` for the current local broadcast name
- Deprecated `device_identity.json` is migration input only; after settings identity is valid, stale JSON is deleted
- Paired-device `display_name` is captured at pairing time and does not auto-update from later discovery name changes
- Visible Settings rows do not combine display names with UUID hashes; UUIDs remain storage/route identity

## Primary UI Surfaces

- `src/views/SettingsView.vue` — the devices list and incoming pair requests
- `src/components/SettingsView/PairDeviceDialog.vue` — pairing, as a modal; replaces the former `/settings/add-device` route
- `src/views/DeviceView.vue` — one paired device: channels, shared spaces, sync queue, unlink
- `src/components/DeviceView/ChannelSetupDialog.vue` — adding or re-establishing a channel on an already-paired device, with no passcode step
- `src/views/DeviceView.vue`

## Related Feature

- `specs/space-sync/README.md`
- `specs/e2e/transports.md` for the E2E topology-to-verification matrix

## Wiki Links

- `~/projects/fini-wiki/pages/concepts/DeviceConnection.md` when present
- `~/projects/fini-wiki/pages/concepts/device-sync-architecture.md` for architecture history
