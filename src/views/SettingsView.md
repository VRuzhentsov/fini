# SettingsView

Route: `/settings`. Tab: Settings. See [[App.md]].

## Concept

Configuration screen with inline sections and drill-down routes for device flows.

Device routes:

- `/settings/add-device` -> [[AddDeviceView]]
- `/settings/device/:id` -> [[DeviceView]]

## Sections

### Spaces
Manage named contexts that quests can belong to. See [[spec/Space]].

- List all spaces with inline edit and delete (built-ins `1/2/3` cannot be deleted)
- Add new space by name

Notes:
- Spaces management lives only in Settings (no dedicated Spaces tab)
- Settings search is planned later and should index this section

### Devices

Device sync settings entry point. See [[spec/DeviceSync]].

- `DeviceList` is visible inline on `/settings`
- Device rows navigate to [[DeviceView]]
- `Add device` row is always last and navigates to [[AddDeviceView]]
- Device status uses green/gray presence indicator
- Mapping behavior in device details is currently `TBD` (read-only placeholder)

### Voice Model
Manages the on-device ASR model (`sherpa-onnx-streaming-zipformer-small-en`, ~60 MB).

| State | UI |
|---|---|
| Not downloaded | Amber "Not downloaded" badge + enabled Download button |
| Downloading | Progress bar + label (`file (N/M) pct%`) + disabled button |
| Downloaded | Green "Ready" badge + disabled "Downloaded" button |

Progress label format: `Downloading <filename> (<file_index+1>/<file_count>) <pct>%`. When percent is `-1` (unknown), shows `…`.

## Dependencies

| Dep | Role |
|---|---|
| [[useModelDownload]] | Download state, progress, error, start/check |
| [[space.ts]] | `fetchSpaces`, `createSpace`, `updateSpace`, `deleteSpace` |
| Device sync runtime adapter (`TBD`) | Paired devices list, presence state, and pairing actions |

## Future sections

### Theme

MVP stores a theme-ready architecture contract (token JSON + CSS variables, including typography tokens) even if user-facing custom theme import UI is added later.
