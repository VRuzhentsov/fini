# SettingsView

Route: `/settings`. Tab: Settings. See [[App.md]].

## Concept

Configuration screen. Sections are presented as an accordion — one section open at a time.

## Sections

### Spaces
Manage named contexts that quests can belong to. See [[Space]].

- List all spaces with inline edit and delete (default space id=1 cannot be deleted)
- Add new space by name

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
