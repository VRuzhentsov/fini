# DeviceView

Route: `/settings/device/:id`. Parent: [[SettingsView]].

## Concept

Detail view for a paired device.

## Current scope

- Show paired device identity (`display_name`, UUID suffix)
- Show presence status (green/gray)
- Show mapping placeholder (`Mapped spaces: TBD`)
- Provide `Unpair` action

## Unpair behavior

- Requires confirmation dialog
- Removes device from DeviceList immediately
- Stops future sync with that device
- Keeps already synced local data

## Deferred

- Editable display name
- Mapping configuration details and direction semantics
