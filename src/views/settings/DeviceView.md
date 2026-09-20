# DeviceView

Route: `/settings/device/:id`. Parent: [[SettingsView]].

## Concept

Everything about one paired device. Redesigned by `docs/adr/0007-channels-connect-and-stay-in-sync.md`.

The page exists to answer three questions at a glance, and every section is there to serve one of them:

- **Is it connected?** — the Channels list
- **Has my stuff synced?** — the Sync queue section
- **If not, why not?** — the reason on each channel row

Features: `specs/device-connect/README.md`, `specs/space-sync/README.md`.

## Current scope

- Device display name as the page header; UUID remains the route/storage identity and is never shown in a normal Settings row
- **Channels** — one [[ChannelRow]] per channel (Network, Bluetooth)
- **Shared spaces** — editable mapped spaces for this pair, with last synced date and time
- **Sync queue** — [[SyncQueueSection]]
- **Set up Bluetooth** — opens [[ChannelSetupDialog]] when Bluetooth isn't on for this pair
- `Unlink` action, last on the page, as a text button

## Channels

Each row carries live state, the reason it isn't connected, a last connected/synced stamp, a star for the primary channel, and its own on/off switch.

- **Row state** is derived by `channelRowState` in [[channelStatusCodes]]: `off` / `waiting` / `down` / `connecting` / `fading` / `connected`. It combines the backend's `RowState` with the pair's own switch, because "off" is a fact about what the user chose and every other state is a fact about the link.
- **The reason** is plain language and names the device — "Pixel 8 isn't nearby", "Bluetooth is off on this computer". Never a status code, never a bare coloured dot. It lives in the row's information button.
- **`waiting`** ("On, waiting") is the state the user sits in after switching a channel on while this machine's own radio is off. The switch stays on with a gray track, and the channel starts by itself when the radio returns. Its reason is shown without being asked for, since a toast would be gone before the question is.
- **The star** only appears on a channel that is actually connected — offering it on a dead row would promise a switch that does nothing.
- Switching a channel on never fails and never reverts; see the ADR.

## Sync queue

- "Everything synced", with when the last change reached this device, or "N changes waiting"
- Expanded, lists the titles of the Quests waiting, ten at most, then "N+ more"
- Titles resolve for Quests and Spaces only; other entity types are counted but not listed, because the person never named them

## Device Identity

- `peer_device_id` is the stable UUID primary key
- `display_name` is a label captured at pairing time, local to this device
- Duplicate display names are allowed
- Settings UI does not combine display name and UUID into one visible row value

## Mapping behavior

- Mapping is symmetric for the pair (one effective mapping state for both peers)
- Enabling mapping for a space triggers immediate bootstrap sync
- Mapping uses `space_id` identity (not name matching)
- If peer is missing mapped space, it is auto-created with the same id
- Mapping `Personal` (`"1"`) enables owner-scoped [[FocusHistory]] replication between the pair
- Mapped-space rows do not show space UUID hashes; status text occupies the `end` column when applicable

## Unlink behavior

- Requires a confirmation that names the spaces which stop syncing and says nothing is deleted
- Removes device from DeviceList immediately
- Stops future sync with that device
- Keeps already synced local data

## Deferred

- Inline rename of the header — designed, owned by issue #117
- Per-channel delete distinct from the on/off switch; see the ADR for why it is not built
- Mapping presets/templates
