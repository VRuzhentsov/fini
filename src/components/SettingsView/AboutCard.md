# AboutCard

Settings subsection for app metadata and project link.

## Purpose

Show the current app version and a link to the source repository inside [[SettingsView]].

## Props

| Prop | Type | Meaning |
|---|---|---|
| `version` | `string` | App version label shown in the card |
| `sourceUrl` | `string` | External URL for the source code link |

## Layout

```
About
  Version    0.1.7
  Source code ↗
```

## Behaviour

- Version text is read-only
- Source code opens in a new tab/window
