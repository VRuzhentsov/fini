# Plan: a layered `ble-gatt` and Fini on iroh

Written 2026-10-04, to resume work after a context reset. It covers two
repositories: `VRuzhentsov/fini` (this one) and `VRuzhentsov/ble-gatt`.

## Where things stand

### Decisions (read these first)

| Document | Repo | PR | What it decides |
|---|---|---|---|
| ADR-0009 `docs/adr/0009-agpl-and-iroh-communication-stack.md` | fini | #188 (draft) | D1 AGPL + trademark, as Signal. D2 iroh replaces the whole communication layer. D3 Bluetooth is our own iroh transport on `ble-gatt`, no `blew` / `iroh-ble-transport`. D4 Fini decides when Bluetooth scans and advertises (ADR-0008 D0/D8/D12). D5 no AGPL code in `ble-gatt`. D6 who owns which layer. D7 what happens to Channel / DataLink / PeerSession / PeerFrame. |
| ADR-0006 `docs/adr/0006-iroh-transport-crate.md` | ble-gatt | #26 (draft) | `ble-gatt-iroh` crate: iroh `CustomTransport` over the datagram channel; the caller owns the radio (`attach`, `set_peer_address`, `Dialer`); transport id `0x424C4547`; GATT only. |
| ADR-0007 `docs/adr/0007-layered-architecture.md` | ble-gatt | #27 (draft) | D1 core up to the datagram profile + power profiles. D2 crates: `ble-gatt` core, `ble-gatt-iroh`, `tauri-plugin-ble-gatt`. D3 `dyn` ports + dependency injection. D4 one object per role. D5 connection state published on a `watch` channel + `CancellationToken`. D6 handles that stop on drop (as `bluer`). D7 an `Adapter` object (as `bluest`). D8 advisory power profiles (as bitchat). D9 Kotlin ships with the Tauri plugin, never vendored. D10 the app picks its network stack. D11 tests. D12 Tokio only. D13 layers named after Clean Architecture (Entities, Use Cases, Interface Adapters, Frameworks & Drivers), modules after the Bluetooth spec and `embedded-hal`; `L<n>` means OSI only. |
| `docs/architecture.md` | ble-gatt | #27 | Layer map (Clean Architecture names), OSI mapping, owners, crates, target module layout, how to design a module across platforms. |
| `docs/glossary.md` | ble-gatt | #27 | One meaning per term, with synonyms. |
| `AGENTS.md` | both | #188, #27 | How to ask the user for a decision (define terms, options with examples and full tradeoffs, name the existing project followed). |

### Branches and PRs

| Repo | Branch | PR | Contents |
|---|---|---|---|
| fini | `claude/chat-session-0cecli` | #188 draft | ADR-0009, AGENTS.md rule, this plan |
| ble-gatt | `feat/ble-gatt-iroh` | #26 draft, CI green | `ble-gatt-iroh` crate, ADR-0006, mock-radio QUIC tests |
| ble-gatt | `docs/architecture-layers` | #27 draft | ADR-0007, architecture.md, glossary.md, AGENTS.md |

### Progress (2026-10-05)

| Phase | PR | State |
|---|---|---|
| B1 plugin Rust API, permissions, JNI check | ble-gatt #28 | draft, CI green |
| F1 Fini on the plugin | fini #189 (pinned to ble-gatt `4d3912c`) | draft, Android emulator E2E green |
| B2 layer modules | ble-gatt #29 (on #28) | draft |
| B3 Adapter, roles, Connection state, ServerHandle | ble-gatt #30 (on #29) | draft |
| B4 power profiles | ble-gatt #31 (on #30) | draft |
| F0 AGPL license | fini #190 | draft |
| F2 Fini onto the role API | — | deferred: F4 replaces `channel/ble.rs` |
| F-spike | — | needs hardware |
| F3–F5 | — | need open questions 1–3 |
| B5 Windows, Apple | — | not started |

Merge order for ble-gatt: #28, #29, #30, #31 (stacked). #26 and #28 both
edit `ci.yml`; the second to merge resolves that conflict. After #28
merges, repin Fini #189 to the merge commit.

Merged earlier: fini #185 (ADR-0008), fini #187 (Bluetooth pairing fixes,
pinned to ble-gatt `0236b84`), ble-gatt #24 (late scan properties, LE
transport). Release v0.3.14 shipped.

## Rules that apply throughout

- No `Co-Authored-By` or AI attribution in commits or PRs; remove any footer a
  tool appends. Commits are authored as Vitalii Ruzhentsov.
- Never merge a PR. Never comment on a PR unless asked. Releases only through
  the Release Button action.
- Prefer Makefile targets. Load `fini-dev` (and `fini-test` for tests) before
  Fini work.
- Ask decisions with the question tool, following the "Asking the user for a
  decision" rules in `AGENTS.md`. Answer in the user's language.
- `ble-gatt` stays MIT: take ideas only from AGPL/GPL/BUSL projects; copy
  code only from MIT/Apache/BSD ones, with attribution.
- Prefer existing designs over new ones; name the project followed.
- `L` with a number means an OSI layer only, in both repositories.
  `ble-gatt`'s layers are Entities, Use Cases, Interface Adapters,
  Frameworks & Drivers (Clean Architecture).
- Verify before asserting; report what was not verified.
- Disk is limited (~4 GB free at the time of writing): build with
  `CARGO_INCREMENTAL=0`, clear `target/*/incremental` when it fills.

## Open questions (need the user)

1. Does the iroh `EndpointId` replace Fini's `device_id`, and how do existing
   pairings migrate (move the key, or pair again)? Blocks Fini phase F3.
2. What remains of ADR-0008's channel model once iroh selects paths:
   per-channel `None`/`Off`/`On`, init by mutual hello, the primary channel,
   per-channel presence. Blocks F3–F5.
3. iroh relays: off (`presets::N0DisableRelay`, local-first) or allowed?
4. A CLA for outside contributors (needed before accepting PRs once AGPL).

## Phases

Each phase is its own PR (or PR pair), keeps existing tests green, and ends
with the verification listed.

### B1 — `tauri-plugin-ble-gatt` ready for Fini (ble-gatt) and F1 — Fini adopts it (fini)

Goal: ADR-0007 D9. Fini stops vendoring `BleGattBridge.kt` and its own
Android context bridging.

ble-gatt:
- Compare `tauri-plugin-ble-gatt/android/.../BleGattBridge.kt` with Fini's
  `src-tauri/gen/android/app/src/main/kotlin/dev/blegatt/BleGattBridge.kt`
  (identical at ble-gatt `0236b84`).
- Make the plugin cover what Fini does in `src-tauri/src/services/android_context.rs`
  and `channel/ble.rs` lazy backend: install `ndk-context` once, resolve app
  classes through the app class loader, lazy backend creation after the
  WebView is up.
- Add a Rust-side API like `tauri-plugin-blew`'s: permission status, request
  and events; `request_enable_bluetooth`; adapter events; emulator detection.
  Ideas only (blew is AGPL).
- Add a test that every Kotlin `external fun` has a matching Rust
  `extern "C"` and back (idea from blew's `jni_parity.rs`; write our own).

fini:
- Depend on `tauri-plugin-ble-gatt`, register it, remove the vendored Kotlin
  and `android_context.rs` parts the plugin now owns.
- Verify: `cargo test --features ui-plane,devtools`, `cargo check --bin fini
  --features cli-plane`, Android build (`make android-debug-deploy`), and on
  hardware: pairing and adding a Bluetooth channel (spec
  `specs/e2e/actors/tests/bluetooth-channel-cycle.spec.ts` against real
  devices).

### B2 — Core modules follow the layer map (ble-gatt)

Move code without changing behaviour (ADR-0007 D13):
`backend/{linux,android,mock}` → `drivers/`, the `Backend` trait → `hal/`,
`backend/link_state.rs` + `peer_link.rs` → `connection/`, `datagram/` →
`profile/`, `models.rs` + `error.rs` → `entities/`. Keep public re-exports so Fini and
`ble-gatt-iroh` still build. Update `docs/architecture.md`.
Verify: `cargo test -p ble-gatt -p ble-gatt-iroh`, Fini builds against the
branch.

### B3 — Roles, Adapter, handles, published state (ble-gatt), F2 — Fini migrates (fini)

ADR-0007 D4–D7:
- `Adapter` (as `bluest`): on/off, `watch` of power state, capabilities;
  roles created from it.
- `Central`, `Peripheral`, `Advertiser` objects over per-role ports (as the
  platforms and `blew` / Nordic split them).
- Handles that stop on drop for advertising, GATT server and scan (as
  `bluer`'s `AdvertisementHandle` / `ApplicationHandle`).
- Each connection publishes `LinkState` on `tokio::sync::watch`, plus
  `cancelled()` → `tokio_util::sync::CancellationToken`.
- Port `ble-gatt-iroh`, the mock and the mock broker.
- Fini: move `channel/ble.rs` onto the new API. **Decided: deferred** — F4
  replaces `channel/ble.rs` with `ble-gatt-iroh`, so migrating it first
  would be thrown away.
Verify: unit tests per role on the mock; mock-broker e2e lane
(`npm run test:e2e:ci:ble`) in Fini.

### B4 — Power profiles (ble-gatt), F-power — Fini applies them

ADR-0007 D8, following bitchat's `PowerManager` / `PowerProfileResolver`
(ideas only, GPL): inputs foreground/background, battery band, charging,
peers nearby → scan on/off times, connection limit. Advisory. Fini's search
coordinator (ADR-0008 D12) reads the profile.

### F0 — License (fini)

ADR-0009 plan step 1: `LICENSE` (AGPL-3.0-or-later), trademark note, source
link in About. Independent of the rest; can go first. CLA per open
question 4.

### F-spike — iroh on hardware (fini, throwaway branch)

ADR-0009 plan step 2, after B1 if possible: iroh on Android (APK size added),
QUIC over `ble-gatt-iroh` between laptop and Pixel (handshake time,
throughput over real GATT), Fini switching scanning/advertising underneath a
live transport. Results go into ADR-0009 Observations.

### F3 — Network over iroh (fini)

Needs open questions 1–3. Device identity = iroh key; pairing and sync move
to QUIC streams under Fini's ALPN; remove `tcp_ws`, Fini's auth and
`DataLink` for Network; mDNS via `iroh-mdns-address-lookup`. Update
`docs/glossary.md` (ADR-0009 D7) and `communication/README.md`.

### F4 — Bluetooth over iroh (fini)

`ble-gatt-iroh` (after #26 merges) replaces `channel/ble.rs` and
`BleDataLink`; Fini's search coordinator supplies addresses
(`set_peer_address`) and accepted channels (`attach`) and keeps its scan and
advertising policy. Keep the `actors-ble` mock-broker lane.

### F5 — Rework ADR-0008 (fini)

Settle open question 2 and rewrite the parts of ADR-0008 whose mechanics iroh
replaced.

### B5 — Windows, then Apple (ble-gatt)

Windows backend on WinRT: central as `bluest` / `btleplug` do it (MIT/Apache,
BSD), peripheral from `ble-peripheral-rust`'s `winrt` module (MIT, code may
be copied with attribution). Apple afterwards.

## Reference material

Studied on 2026-10-04 (clones lived in the session scratchpad and are gone;
re-clone shallow if needed):

| Project | License | Borrow |
|---|---|---|
| Nordic Kotlin-BLE-Library | BSD-3 | role × layer module split; `Environment`; `Peripheral` + `Executor`; `profile()` |
| Kable | Apache-2.0 | `connect()` → scope; `observe()` that survives reconnects; `state: StateFlow`; Rust core via uniffi (`kable-btleplug-ffi`) |
| bluest | MIT/Apache | `Adapter` newtype over cfg-selected `sys`; Windows and Android backends |
| btleplug | BSD-3 | `Central`/`Peripheral` traits; WinRT backend |
| bluer | BSD-2 | RAII `AdvertisementHandle` / `ApplicationHandle` |
| ble-peripheral-rust | MIT | WinRT peripheral |
| blew, iroh-ble-transport | AGPL | ideas: per-device GATT op queue, JNI parity test, Kotlin shipped via `links`, prefix keys for MAC rotation, lifecycle ids |
| bitchat-android | GPLv3 | ideas: `PowerManager` profiles, transport-neutral `MeshService` facade |
| SimpleBLE | BUSL-1.1 | ideas: simulator backend, hardware-in-the-loop rig |
| iroh | MIT/Apache | `CustomTransport` (`unstable-custom-transports`, iroh 1.3), `test_utils/test_transport.rs` |
