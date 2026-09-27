# DeviceView

Route: `/settings/device/:id`. Parent: [[SettingsView]].

## Concept

Everything about one paired device. Redesigned by `docs/adr/0007-channels-connect-and-stay-in-sync.md`; channels follow `docs/adr/0008-channel-state-machine.md`.

The page exists to answer three questions at a glance, and every section is there to serve one of them:

- **Is it connected?** — the Channels list
- **Has my stuff synced?** — the Sync queue section
- **If not, why not?** — the ⓘ on an orange channel row

Features: `specs/device-connect/README.md`, `specs/space-sync/README.md`.

## Current scope

- Device display name as the page header; UUID remains the route/storage identity and is never shown in a normal Settings row
- **Channels** — one [[ChannelRow]] per channel (Network, Bluetooth)
- **Shared spaces** — editable mapped spaces for this pair, with last synced date and time
- **Sync queue** — [[SyncQueueSection]]
- `Unlink` action, last on the page, as a text button

## Channels

Each row is a colour, a name and controls. The backend decides the colour (ADR-0008 D19); the row only draws it:

| Colour | State | Meaning | Controls |
|---|---|---|---|
| green | on | the peer was heard on this channel within its timeout, or an exchange is open | switch |
| grey | on | the peer was not heard lately | switch |
| orange | on | a problem on this device (radio unavailable, network unreachable) | switch, ⓘ |
| empty circle | off | switched off by the person | switch, unlink |
| no dot | none | the channel does not exist | Add |

- **ⓘ** appears only on orange, and opens a popup naming this device's problem. A peer that is simply away is grey, not a problem, and has nothing to explain. Nothing explanatory is rendered inline.
- **Add** opens [[DeviceSetupDialog]] for that channel. The channel exists only once both devices confirmed each other there (D1); closing the dialog before that writes nothing.
- **The switch** turns an existing channel on or off. Turning on is refused when the channel cannot work on this device (D6), with the reason shown.
- **Unlink** (trash) exists only on an off channel: forgetting a channel is a second, deliberate act (D14). It deletes the channel and tells the peer.
- **Presence** is searched for only while this page is open (D12); the rows are re-read every 5 seconds.

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
- Mapping presets/templates
