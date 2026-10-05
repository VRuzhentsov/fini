# AboutCard

Settings subsection for app metadata and project link.

## Purpose

Show the current app version, a link to the source repository and the license inside [[SettingsView]]. The source link is what the AGPL asks of a program people use: a way to get its source (ADR-0009 D1).

## Props

| Prop | Type | Meaning |
|---|---|---|
| `version` | `string` | App version label shown in the card |
| `sourceUrl` | `string` | External URL for the source code link |
| `license` | `string` | SPDX identifier of the license, shown in the row |
| `licenseUrl` | `string` | External URL of the license text |

## Layout

```
About
  Version    0.1.7
  Source code ↗
  License    AGPL-3.0-or-later ↗
```

Rows use [[SettingsListItem]] inside [[SettingsListGroup]].

## Behaviour

- Version text is read-only
- Source code and License open in a new tab/window
