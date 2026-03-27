# Sync Devices Design (Issue #4)

Date: 2026-03-23
Status: Before implementation baseline
Scope: Spec-only decisions for MVP.1 device sync

## Goal

Lock product and protocol decisions before coding Device Sync.

## Locked decisions

- New dedicated spec: `spec/DeviceSync.md`
- Authority split:
  - `spec/Network.md` owns transport-level model
  - `spec/DeviceSync.md` owns UX behavior and pairing flow

### Settings UX / IA

- Routes:
  - `/settings`
  - `/settings/add-device`
  - `/settings/device/:id`
- `/settings` sections order:
  1. Spaces
  2. Devices
  3. Voice Model
- `DeviceList` is inline on `/settings`
- `Add device` row is always last

### Add-device behavior

- Pairing requests are processed only when both devices are in Add Device mode
- Add Device discovery refresh cadence: every 5s
- Candidate list rules:
  - newest seen first
  - dedup by `device_id`
  - hide already paired devices
- Leaving Add Device view cancels pending requests immediately

### Pairing behavior

- 6-digit passcode is mandatory
- Sender role = first click timestamp
- Tie-breaker = lower `device_id`
- Receiver gets incoming request sheet
- Sender sees code only after receiver accepts
- 3 wrong attempts per remote device -> 60s cooldown
- Pending request timeout: 60s
- Code does not expire while Add Device view remains open

### Device detail behavior

- `/settings/device/:id` shows status and `Unpair`
- Status indicator: green/gray
- `Mapped spaces` section is placeholder (`TBD`)
- Unpair requires confirmation
- Unpair removes device from list immediately
- Unpair keeps already synced local data

### Presence / sync constraints

- Normal presence heartbeat: 60s
- Offline after 2 missed heartbeats (120s)
- Platform parity required across app targets
- Offline queue durability required (survive restart/crash)
- Queue schema is deferred
- Mapping semantics are deferred (`TBD`)

## Constraints

- No polished UX.
- No cloud relay/account.
- No implementation in this phase.

## Security phase policy

- MVP.1 requires passcode-gated pairing
- Transport encryption is deferred beyond MVP.1
- Later phase makes encrypted transport mandatory

## Start-implementation readiness

- Specs are aligned in:
  - `spec/DeviceSync.md`
  - `spec/Network.md`
  - `src/views/SettingsView.md`
  - `src/views/AddDeviceView.md`
  - `src/views/DeviceView.md`
- Implementation branch can start after this commit

## Follow-up backlog

- Mapping semantics design session (`TBD`)
- Durable queue schema details
- Transport encryption implementation phase
