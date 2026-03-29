# Sync Devices Design (Issue #4) - Archived Snapshot

Date: 2026-03-23
Status: superseded

This baseline was replaced by `docs/plans/2026-03-29-device-synchronizations-design.md`.

## Why superseded

- `device_sync` was split into two services:
  - [[DeviceConnection]]
  - [[SpaceSync]]
- `Main` terminology moved to `Focus` with [[FocusHistory]] model.
- Mapping semantics and durable replay model are now fully defined.

## Current source of truth

- `spec/DeviceConnection.md`
- `spec/SpaceSync.md`
- `spec/FocusHistory.md`
- `spec/Network.md`
- `src/views/SettingsView.md`
- `src/views/AddDeviceView.md`
- `src/views/DeviceView.md`

## Unresolved questions

- none
