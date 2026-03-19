# Network & Local Sync

Fini is local-first. All data lives on-device. Network features are opt-in and operate entirely within the local network — no cloud, no accounts.

## Goals

- Devices on the same LAN can discover each other automatically — no IP addresses, no router config.
- Before any data is shared, devices must **pair** explicitly. Discovery is passive; sync is opt-in.
- After pairing, the user selects which [[Space]]s to synchronise. Only chosen spaces are shared.
- Pairing is a one-time action per device pair, secured by mutual confirmation. It survives app restarts.
- MCP clients (Claude Desktop, opencode) on any paired device can read and write quests.
- Unpairing immediately stops sync and revokes access.

## Entry points

Fini data can be accessed from multiple entry points:

- **GUI app** — the primary experience on desktop and mobile.
- **MCP server** — allows AI clients to read and write quests.
- **Headless** — running without a UI, suitable for an always-on shared node on a home network or NAS.

All entry points operate on the same data. A change made through any one of them should be reflected in all others without requiring a manual refresh.

## Local network discovery

Fini instances on the same LAN advertise their presence so other devices can find them without manual configuration. The exact discovery mechanism is an implementation detail; the intent is zero-config — open the app, see nearby devices.

### Pairing

Discovered devices are visible but inert until paired. Pairing requires confirmation on both sides. Once paired, sync begins automatically on every subsequent connection.

The user can:

- **Pair** — initiate pairing with a nearby device.
- **Unpair** — revoke access and stop sync.
- **Ignore** — hide the device from the nearby list.

### Space selection

After pairing, the user chooses which spaces to synchronise. Spaces are matched by name across devices — "Personal" on one device maps to "Personal" on the other.

To make this reliable, certain spaces are built-in and non-deletable: **Personal** and **Family**. These exist on every Fini install under the same name, giving devices a guaranteed common ground. User-created spaces can also be synced if both devices have a space with the same name.

### Sync

Quests in selected spaces are kept in sync across paired devices in real time. Conflicts are resolved by last-write-wins based on `updated_at` (UTC).

## Security

- LAN sharing is off by default. The user opts in explicitly.
- All peer-to-peer communication is encrypted in transit.
- All data at rest is encrypted.
- Pairing is authenticated — both devices must confirm before any data flows. Unpaired devices receive no data, even if they can discover the instance.
