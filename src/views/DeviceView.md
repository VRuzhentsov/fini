# DeviceView

Route: `/settings/device/:id`. Parent: [[SettingsView]].

## Concept

Detail view for a paired device.

Feature: `specs/space-sync/README.md`.

## Current scope

- Show paired device display name and presence status; UUID remains the route/storage identity and is not shown in normal Settings rows
- Show presence status (green/gray)
- Show editable mapped spaces for this pair
- Show last synced date and time for mapped spaces
- Provide `Unpair` action

## Device Identity

- `peer_device_id` is the stable UUID primary key
- `display_name` is a label captured at pairing time
- Duplicate display names are allowed
- Settings UI does not combine display name and UUID into one visible row value

## Mapping behavior

- Mapping is symmetric for the pair (one effective mapping state for both peers)
- Enabling mapping for a space triggers immediate bootstrap sync
- Mapping uses `space_id` identity (not name matching)
- If peer is missing mapped space, it is auto-created with the same id
- Mapping `Personal` (`"1"`) enables owner-scoped [[FocusHistory]] replication between the pair
- Mapped-space rows do not show space UUID hashes; status text occupies the `end` column when applicable

## Unpair behavior

- Requires confirmation dialog
- Removes device from DeviceList immediately
- Stops future sync with that device
- Keeps already synced local data

## Deferred

- Editable display name
- Mapping presets/templates
