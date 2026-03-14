# SettingsView

Route: `/settings`. Tab: Settings. See [[App.md]].

## Concept

Configuration screen. Currently contains only the voice model section.

## Sections

### Voice Model
Manages the on-device ASR model (`sherpa-onnx-streaming-zipformer-small-en`, ~60 MB).

| State | UI |
|---|---|
| Not downloaded | Amber "Not downloaded" badge + enabled Download button |
| Downloading | Progress bar + label (`file (N/M) pct%`) + disabled button |
| Downloaded | Green "Ready" badge + disabled "Downloaded" button |

Progress label format: `Downloading <filename> (<file_index+1>/<file_count>) <pct>%`. When percent is `-1` (unknown), shows `…`.

## State

Uses [[useModelDownload]]. Calls `checkDownloaded()` on mount.

## Dependencies

| Dep | Role |
|---|---|
| [[useModelDownload]] | Download state, progress, error, start/check |
