# Transport topologies and where they're verified

Companion to `docs/adr/0001-transport-neutral-peer-protocol.md`. Issue #25
asked for CI coverage across android-linux, android-android, linux-linux
(and the user's request extended this to windows-android, deferred — see
below). GitHub-hosted CI runners have no Bluetooth hardware and cannot
reliably run two Android emulators in one job, so "coverage" means
different things at different layers. This table is the honest mapping.

| Layer | What it proves | Runs on |
|---|---|---|
| Rust integration tests (`services::transport::tests`) | Transport-neutral: network-first selection, auth-gate rejects un-authed inbound, sticky single-session invariant, Bluetooth-fallback-role handoff, no duplicated/lost events. Runtime-agnostic — identical Rust runs on every OS, so this is the proof that holds for android-linux, android-android, linux-linux, and windows-android alike; there is no "android-ness" at the protocol layer. | Every PR, GitHub-hosted (`make pr-gate-be-unit`) |
| Playwright `actors` project, `transport-selection.spec.ts` | Real app binaries, real network transport: paired actors with network available claim `tcp_ws`, never fall back. | Every PR, GitHub-hosted (`make pr-gate-e2e`) |
| Playwright `actors-sim` project, `peer-sync-over-sim.spec.ts` | Real app binaries, network transport genuinely disabled (`FINI_DISCOVERY_DISABLED=1`): session claims `sim` (playing the Bluetooth-fallback role), an approved Space replicates over it, no new consent prompt, session stays sticky across ticks. | Every PR, GitHub-hosted (`make pr-gate-e2e`, chained in `scripts/e2e-runner.sh`) |
| `android-emulator-e2e` CI job | Single Android emulator: app boots, notification channel exists. Not a cross-device sync test. | Every PR, GitHub-hosted |
| Real Bluetooth, real second device | The topologies as literally described in the ticket. | Local only (`make e2e-bt-local`, PR B) — no device lab, no self-hosted runner |

## Why real cross-device/cross-radio topologies aren't a PR gate

- GitHub-hosted runners have no Bluetooth radio. There is no way to run a
  real BLE/RFCOMM link between two processes on the same runner, let alone
  across two runners.
- The Android emulator needs KVM, only available on Linux GitHub-hosted
  runners; running two emulators in one job to sync with each other is
  possible but heavy and flaky, and still wouldn't have a real radio.
- Windows GitHub-hosted runners have no Android emulator support at all, so
  windows-android has no real-hardware path on GitHub-hosted CI regardless
  of transport work. It is out of scope for this PR (the ticket itself
  scopes Bluetooth to Android + Linux); the transport port and topology
  table both leave room for it as a future ticket.

## What closes the gap between "CI-proven" and "the ticket's real topologies"

The Rust integration test layer is deliberately runtime-agnostic *by
construction* — `space_sync::session` and `services::transport` have no
`cfg(target_os = ...)` branches — so once the real Bluetooth adapter (PR B)
exists, the exact same tests, run on the exact same CI, prove the exact same
selection/auth-gate/sticky-handoff semantics for it. What CI cannot do is
prove the *radio* works; that is what `make e2e-bt-local` (manual, this
machine + a real Android device) is for, and why it stays out of the PR
gate rather than being faked into looking automated.
