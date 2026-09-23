# Naming

How names are **formed** in Fini: plurals, casing per language, DOM hooks,
command prefixes. Rules about spelling, not about meaning.

What the words *mean* — channel, DataLink, session, primary — is
[`glossary.md`](glossary.md). Reach for that one when choosing *which* word;
reach for this one when writing it down.

When a rule here conflicts with what the code currently says, this file wins
and the code is wrong.

## Module layout

`services/communication/` holds the three things two devices do to reach
each other:

| Module | Owns |
|---|---|
| `pairing` | establishing trust, and the channels a pair has configured |
| `channel` | carrying bytes — the connection code, framing, and the encryption seam |
| `sync` | the application protocol over a channel, and the outbox behind it |

A module is named for the concept it owns, not for the trait it exports:
`channel/encryption.rs` holds `SecureChannel` and `PlaintextChannel`.

ADRs 0001–0006 predate the current module layout and refer to the old
`services/{transport,space_sync,device_connection}/` paths. They are records
of decisions as taken, so they are left as written.

## Plurals

- **Tables are plural** when a row is one countable thing: `quests`,
  `spaces`, `channels`, `paired_devices`. They stay as they read when the
  name is a mass noun or an activity log: `focus_history`, `sync_outbox`,
  `checklist_activity`, `settings`.
- **A directory holding many things is plural**: `services/`, `views/`,
  `components/`, `migrations/`.
- **A module named for one concept is singular**: `quest.rs`, `channel/`,
  `sync/`, `pairing/`.
- **A folder that groups a screen's parts is singular**, after the thing it
  belongs to: `components/settings/`, `components/settings/device/`.
- **Types are singular**: `Quest`, `ChannelKind`, `PairedDevice`. A type
  naming a collection says so: `SyncQueueSummary`.

## Case

- **Files**: kebab-case wherever the framework allows it. Vue SFCs and Rust
  modules are the exceptions that don't — SFCs are `PascalCase.vue` and Rust
  modules are `snake_case.rs`, matching their ecosystems.
- **Rust**: `snake_case` items, `PascalCase` types, `SCREAMING_SNAKE_CASE`
  constants.
- **TypeScript**: `camelCase` locals and functions, `PascalCase` types.
  Fields that cross the Tauri boundary keep the Rust spelling
  (`peer_device_id`, `channel_kind`) rather than being converted — one name
  per field, on both sides of the bridge.
- **DOM hooks**: `data-testid` and `data-*` attributes are kebab-case and
  name the thing, not its position: `data-testid="channel-status-row"`,
  `data-channel-kind="bluetooth"`.
- **Tauri commands**: `snake_case`, prefixed by the surface that owns them
  (`space_sync_*`, `device_connection_*`). The `device_connection_*` prefix
  predates `communication/pairing/` and has not been renamed; that is a
  known follow-up, deliberately left out of the vocabulary change so the
  rename stayed reviewable.
