# 0009 — AGPL license and an iroh-based communication stack

## Status

Proposed. A plan; nothing in it is implemented yet.

## Context

Fini connects paired devices over its own stack. Network is TCP and
WebSocket (`channel/tcp_ws.rs`); Bluetooth is our `ble-gatt` crate
(`channel/ble.rs`). On top of both sit our own frame codec, hello-based
channel init, exchanges with acks, a control outbox, presence, and a
search coordinator (ADR-0008). Peer authentication is still open (D16,
#184).

Three projects were studied for this ADR, from their repositories as of
2026-10-04:

| Project | License | State | Role |
|---|---|---|---|
| [iroh](https://github.com/n0-computer/iroh) | MIT OR Apache-2.0 | 1.3, since 2023, ~2600 commits, a company behind it | Dial by public key over QUIC: TLS 1.3 authentication and encryption, streams, path selection, relays and hole punching, custom transports |
| [iroh-ble-transport](https://github.com/mcginty/iroh-ble-transport) | AGPL-3.0-or-later | 0.5.1-beta, since 2026-04, one author, "experimental" | BLE as an iroh custom transport, built on `blew` |
| [blew](https://github.com/mcginty/blew) | AGPL-3.0-or-later | 0.5.1-beta, since 2026-04, one author, "experimental" | BLE central and peripheral, L2CAP, on Linux, Android, macOS and iOS |

`ble-gatt` is also meant for closed-source projects, so it must stay MIT and
must not take a dependency on, or code from, the two AGPL projects.

## Decisions

**D1 — Fini is licensed AGPL-3.0-or-later, following Signal.** The source
stays public, and anyone may fork it under the same license. The name and
icon "Fini" are protected as a trademark, so forks must rebrand.

**D2 — iroh carries the bytes; ADR-0008 keeps deciding which channel.**
Revised by D8–D10. Each channel kind gets its own iroh `Endpoint`, all with
the device's one Ed25519 key, each restricted to one path:

- Network: iroh's IP transport only. Addresses come from Fini's own presence
  beacons (ADR-0008), handed to iroh through its `MemoryLookup`.
- Bluetooth: our own iroh custom transport only (D3); IP transports cleared.

What goes away: `tcp_ws` (the WebSocket server and client) and Fini's
unauthenticated `Auth` check. Authentication and encryption come from iroh's
TLS 1.3 with the device's key, which closes D16 / #184. `PeerFrame`s travel
on a QUIC stream under Fini's ALPN; `DataLink` stays as the seam the
application protocol talks to (D9).

**D3 — Bluetooth is our own iroh transport on `ble-gatt`, not
`iroh-ble-transport` or `blew`.** It is a new MIT crate, `ble-gatt-iroh`, in
the `ble-gatt` workspace, next to `ble-gatt` and `tauri-plugin-ble-gatt`. It
implements iroh's `CustomTransport` and carries QUIC datagrams over
`ble-gatt`'s GATT datagram channel. The `ble-gatt` core crate stays free of
any iroh dependency.

GATT only, no L2CAP. L2CAP CoC on Android needs API 29+ while Fini's
`minSdk` is 24, so GATT would have to stay as a second path anyway, and the
slow part today is discovery and dialling, not throughput.

**D4 — Fini decides when Bluetooth scans and advertises.** ADR-0008 D0, D8 and
D12 stand: Bluetooth works only when there is a reason to. The transport
crate exposes discovery and advertising control to its caller rather than
scanning on its own. `ble-gatt` offers power profiles as advice
(`ble-gatt` ADR-0007 D8); Fini applies them under its own rules.

**D5 — No AGPL code in `ble-gatt`.** `iroh-ble-transport` and `blew` may be read
for ideas, but no code is copied from them, and they are never added as
dependencies.

**D6 — Who owns which layer.** The layers are defined in `ble-gatt`'s
`docs/architecture.md` and named after Clean Architecture (`ble-gatt`
ADR-0007 D13):

| Layer | Owner |
|---|---|
| Entities, Use Cases, Frameworks & Drivers: platform drivers, BLE roles, connections, the datagram profile, peer identity, power profiles | `ble-gatt` core crate |
| Interface Adapter for iroh | `ble-gatt-iroh`. All iroh coupling on the Bluetooth side is in this crate. |
| Interface Adapter for Tauri: Android bridge, permissions | `tauri-plugin-ble-gatt`, which Fini uses instead of a vendored copy of the Kotlin bridge |
| Application: choosing iroh, sessions, sync, pairing, when the radio works | Fini |

Fini decides that Fini uses iroh. `ble-gatt` takes no position on the network
stack, so moving off iroh means a new adapter crate and changes in Fini, not
in `ble-gatt`.

**D7 — What happens to Fini's communication terms.** Fini's terms
(`docs/glossary.md`, `src-tauri/src/services/communication/README.md`) map
onto the new stack as below. This is the intended direction; each line is
settled in the stage that changes it, and the glossary is updated then.

| Fini term today | Under iroh |
|---|---|
| `Channel` (a row: pair + kind + on/off + primary) | Stays. It is the person's setting, not machinery. It decides which paths Fini lets iroh use for a pair. |
| `DataLink` (one connection's byte pipe: `TcpWsDataLink`, `BleDataLink`) | Stays as the seam (D9); one implementation, `IrohDataLink`, over a QUIC stream on that channel's endpoint. |
| `PeerSession` (an authenticated conversation) | Stays (ADR-0008). Authenticated by TLS: the peer's key must equal the `endpoint_id` stored for the pair (D8). |
| `PeerFrame` (one message) | Stays as the message format, sent over a QUIC stream. |

**Layer numbers.** In Fini and `ble-gatt` documents, `L` with a number
always means an OSI layer. `DataLink` is named after OSI L2; in `ble-gatt`
that work is its datagram profile (`profile/`). `ble-gatt`'s own layers have
Clean Architecture names, not numbers.

On the OSI model, the stack after this ADR looks like this (the full table
is in `ble-gatt` `docs/architecture.md`, "How the layers map onto OSI"):

| OSI layer | Who does it |
|---|---|
| L1 Physical | Bluetooth chip; Wi-Fi or Ethernet |
| L2 Data link | OS Bluetooth stack plus `ble-gatt`; OS network stack |
| L3 Network | iroh: addressing by public key, choosing Bluetooth or IP |
| L4 Transport | QUIC inside iroh |
| L5–L7 | Fini: its ALPN, `PeerFrame`, sync and pairing |

**D8 — `device_id` stays a UUID; the iroh key is pinned next to it.**
`paired_devices` gains `endpoint_id` (the peer's public key). Devices
exchange keys while pairing and check them on every connection, as Signal
keeps an identity key next to the account and SSH keeps `known_hosts`.
`origin_device_id` in existing records stays valid. Pairs made before this
have no key and are paired again (Fini's no-legacy rule during the alpha).

Known limitation: the key pinned is the one the peer proved on the pairing
link, and the six-digit code does not cover it. B sends the code to A over
the network, A shows it, and the user types it on B, so the code proves a
person holds both devices but not that the key is theirs. An active
man-in-the-middle on the LAN or a Bluetooth relay present while pairing can
forward the frames, code included, and have both devices pin its keys.
Before this ADR any device that knew a `device_id` could connect at any
time; now an attacker has to be in the middle at the moment of pairing.
Closing the gap is iroh's own answer to "whose key is this": exchange an
`EndpointTicket` (QR code or copied string) that carries the key out of
band, as `dumbpipe`, `sendme` and Delta Chat's QR pairing do. Planned as a
follow-up.

**D9 — ADR-0008 stays whole.** Per-channel `None`/`Off`/`On` state, init by
mutual hello, the primary channel and per-channel presence keep working as
they do; only what is under `DataLink` changes. This is why D2 uses one
endpoint per channel kind: each connection then has exactly one path, so a
channel's state still describes one real link.

**D10 — iroh relays are off.** `RelayMode::Disabled` on every endpoint: Fini
syncs over the local network and Bluetooth only, as today, with no
third-party server seeing connection metadata.

## Observations

Recorded so they are not rediscovered. None of these is a decision.

- **Ideas from `iroh-ble-transport` worth evaluating.** These are design
  notes, not code to take:
  - addresses keyed by a public-key prefix from the advertisement, so a
    peer whose MAC address rotates is followed without redialling a stale
    one;
  - a lifecycle id on every asynchronous step, so late results from an
    abandoned connection are dropped;
  - one queue per device for connect, disconnect and setup work.
- **Unstable iroh API.** Custom transports are behind iroh's
  `unstable-custom-transports` feature ("may change without notice"), and
  the API has had several breaking changes since March 2026.
- **Bluetooth identity in the advertisement.** Dialling by key means the
  advertisement has to identify the key. A stable identifier is visible to
  anyone nearby; our current 4-byte `device_id` fingerprint already is one.
- **Mock radio.** `ble-gatt`'s mock broker already runs across processes, so
  the `actors-ble` e2e lane can keep using it under the new transport.
- **Toolchain.** iroh needs Rust 1.91; CI uses stable.
- **Platforms.** `ble-gatt` does not support Windows or Apple platforms yet;
  Bluetooth there stays out of scope.

## Plan

1. **License.** Add `LICENSE` (AGPL-3.0-or-later), a trademark note, and a
   source link in the app's About screen. Decide on a CLA before accepting
   outside contributions.
2. **Spike on hardware.** Laptop plus Pixel:
   - iroh on Android, and the APK size it adds;
   - a minimal custom transport carrying QUIC over `ble-gatt`'s datagram
     channel: handshake time and throughput over GATT.
3. **Network over iroh.** Device identity becomes the iroh key, pairing and
   sync move to QUIC streams, and `tcp_ws` and our auth are removed.
4. **The `ble-gatt-iroh` crate** (D3, D4), tested on the mock broker
   and on hardware. First version: `ble-gatt` PR #26. Before Fini adopts it,
   Fini moves onto `tauri-plugin-ble-gatt` instead of its vendored Kotlin
   bridge (`ble-gatt` ADR-0007 D9). The step-by-step plan across both
   repositories is `docs/plans/2026-10-04-ble-gatt-layers-and-iroh.md`.
5. **Bluetooth over iroh in Fini.** The new crate replaces `channel/ble.rs`,
   with Fini's scan and advertising policy on top.
6. **ADR-0008 stays** (D9); record in it that its links now run over iroh.

## Open questions

- A CLA for outside contributors.
- Pairing by iroh ticket instead of the six-digit code (D8's known
  limitation).

Settled on 2026-10-05: identity (D8), ADR-0008's channel model (D9), relays
(D10).
