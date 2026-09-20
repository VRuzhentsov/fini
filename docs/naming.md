# Naming

What things are called in Fini, and how names are formed. When a name here
conflicts with what the code currently says, this file wins and the code is
wrong.

## Vocabulary

### Channels

| Term | Means |
|---|---|
| **channel** | the configured connection between two paired devices — what a pair *has* |
| **channel kind** | the medium a channel uses: `network` or `bluetooth` |
| **primary** | the channel the person chose to carry the traffic (`is_primary` in SQL, because `primary` is a keyword) |
| **transport** | the adapter that actually moves the bytes for a channel |

A channel and a transport are not one-to-one, which is why both words exist.
The Network channel is carried by `tcp_ws` in production and by `sim` under
test; `ChannelKind` has two variants and `TransportKind` has four
(`TcpWs`, `Sim`, `Bluetooth`, `LoRa`). "Transport" is therefore not a synonym
to sprinkle around — it belongs in `services/communication/channel/`, where
the adapters live, and nowhere the person can see. Interfaces, commands,
tables, events and UI copy say *channel*.

`InstallChannel` is unrelated: it is the app-update track (stable/beta).

### Communication

`services/communication/` holds the three things two devices do to reach
each other:

| Module | Owns |
|---|---|
| `pairing` | establishing trust, and the channels a pair has configured |
| `channel` | carrying bytes — the adapters, framing, and the encryption seam |
| `sync` | the application protocol over a channel, and the outbox behind it |

`channel/encryption.rs` is the encryption seam (`SecureChannel`,
`PlaintextChannel`), named for what it is rather than for the trait it
exports.

ADRs 0001–0006 were written before this rename and refer to the old
`services/{transport,space_sync,device_connection}/` paths. They are records
of decisions as taken, so they are left as written; ADR-0007 is where the
current vocabulary is decided.

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
