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

`ble-gatt` exists because, when it was started, no permissively licensed
Rust crate could act as a BLE peripheral on Android. The one crate that
could, `blew`, is AGPL-3.0, which did not fit Fini's licensing at the time
(`ble-gatt` ADR-0001).

Three projects were studied for this ADR, from their repositories as of
2026-10-04:

| Project | License | State | Role |
|---|---|---|---|
| [iroh](https://github.com/n0-computer/iroh) | MIT OR Apache-2.0 | 1.3, since 2023, ~2600 commits, a company behind it | Dial by public key over QUIC: TLS 1.3 authentication and encryption, streams, path selection, relays and hole punching, custom transports |
| [iroh-ble-transport](https://github.com/mcginty/iroh-ble-transport) | AGPL-3.0-or-later | 0.5.1-beta, since 2026-04, one author, "experimental" | BLE as an iroh custom transport. QUIC datagrams over L2CAP CoC, or over GATT with its own selective-repeat ARQ when L2CAP is not available |
| [blew](https://github.com/mcginty/blew) | AGPL-3.0-or-later | 0.5.1-beta, since 2026-04, one author, "experimental" | BLE central and peripheral, L2CAP, on Linux, Android, macOS and iOS, plus a Tauri plugin for the Android glue |

## Decisions

**D1 — Fini is licensed AGPL-3.0-or-later, following Signal.** The source
stays public, and anyone may fork it under the same license. The name and
icon "Fini" are protected as a trademark, so forks must rebrand. This also
makes the AGPL libraries above usable.

**D2 — iroh replaces the whole communication layer.** One iroh `Endpoint` per
device carries both channels:

- Network through iroh's IP transport, with local discovery through mDNS
  (`iroh-mdns-address-lookup`).
- Bluetooth through `iroh-ble-transport` on `blew`.

What goes away: `tcp_ws`, our auth, our frame codec, and the code that picks
a channel and keeps a link to it. Authentication and encryption come from
iroh's TLS 1.3 with the device's Ed25519 key, which closes D16 / #184. Sync
and pairing traffic move to QUIC streams under our own ALPN.

**D3 — Fini, not the transport, decides when Bluetooth scans and
advertises.** ADR-0008 D0, D8 and D12 stand: Bluetooth works only when
there is a reason to. `iroh-ble-transport` takes the `blew` `Central` and
`Peripheral` from the caller (`BleTransport::builder().central(..)
.peripheral(..)`), and Fini keeps them and turns scanning and advertising
on and off itself (`Central::start_scan`/`stop_scan`,
`Peripheral::start_advertising`/`stop_advertising`).

**D4 — `ble-gatt` stays as a possible alternative backend.** A custom iroh
transport on top of `ble-gatt` (MIT) is recorded as a follow-up ticket only,
not planned work. It is the fallback if `blew` or `iroh-ble-transport` does
not work out.

## Observations

Recorded so they are not rediscovered. None of these is a decision.

- **Scan and advertising control in `iroh-ble-transport`.**
  - It starts scanning and advertising as soon as it is built, with
    `ScanFilter::default()`: `ScanMode::LowLatency` and no service filter.
  - It has no API to pause discovery, and its registry does not know when
    the application has stopped the scan.
  - After the adapter is switched off and on, it restarts scanning (with the
    default filter) and advertising by itself, so Fini has to watch
    `adapter_state_changes()` and apply its own policy again.
  - A first-class API for this (a scan filter in the builder, a discovery
    mode switch) may be needed later. Nothing has been proposed upstream.
- **Unstable iroh API.** Custom transports are behind iroh's
  `unstable-custom-transports` feature ("may change without notice"), and
  the API has had several breaking changes since March 2026.
- **Bluetooth identity in the advertisement.** `iroh-ble-transport`
  advertises a service UUID that carries 12 bytes of the device's public key.
  That is a stable identifier anyone nearby can see. Our current
  advertisement carries a 4-byte fingerprint of `device_id`, which is also
  stable.
- **Mock radio.** `blew`'s mock (`testing::MockLink`) runs within one
  process. The `actors-ble` e2e lane runs two `fini-app` processes against a
  shared broker, so it needs another approach.
- **Toolchain.** `iroh-ble-transport` needs Rust 1.95 and iroh needs 1.91.
  CI uses stable.
- **Platforms.** `blew` does not support Windows, and neither does
  `ble-gatt`; Bluetooth on Windows stays out of scope. `blew` adds macOS and
  iOS.
- **Linux and Apple peers.** `blew` warns when BlueZ's `battery`/`deviceinfo`
  plugins or the GATT cache are on, because they can trigger pairing prompts
  with Apple devices. This does not affect Linux to Android.

## Plan

1. **License.** Add `LICENSE` (AGPL-3.0-or-later), a trademark note, and a
   source link in the app's About screen. Decide on a CLA before accepting
   outside contributions.
2. **Spike on hardware.** Laptop plus Pixel:
   - iroh on Android, and the APK size it adds;
   - QUIC over `iroh-ble-transport`, on both the L2CAP and the GATT path:
     handshake time and throughput;
   - Fini switching `blew` scanning and advertising on and off underneath a
     running transport;
   - unlink, reconnect, and the phone's address rotation.
3. **Network over iroh.** Device identity becomes the iroh key, pairing and
   sync move to QUIC streams, and `tcp_ws` and our auth are removed.
4. **Bluetooth over iroh.** `iroh-ble-transport` replaces `channel/ble.rs`
   and the `ble-gatt` dependency, with the D3 controller on top.
5. **Rework ADR-0008** where its mechanics are replaced (see Open
   questions), and replace the `actors-ble` mock lane.

## Open questions

- Does the iroh `EndpointId` replace `device_id`, and how do existing
  pairings migrate?
- What remains of ADR-0008's channel model once iroh selects paths:
  - the per-channel `None`/`Off`/`On` state;
  - init by mutual hello;
  - the primary channel;
  - per-channel presence.
- Relays: off (local-first, `presets::N0DisableRelay`) or allowed?
- A CLA for outside contributors.
