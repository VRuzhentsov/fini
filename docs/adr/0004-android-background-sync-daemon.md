# 0004 — Keeping sync alive in the background on Android

## Status

Accepted, implemented on `feat/external-actors`, **verified on a physical
Pixel 6 Pro** — see Verification below. The daemon does what it was built for.
A separate BLE defect (simultaneous bidirectional dialling) keeps sessions from
*staying* up; that is out of this ADR's scope and recorded at the end.

## Context

Sync ticks were scheduled entirely by the frontend: `startMappingUpdateLoop` in
`src/stores/device.ts` runs `space_sync_tick` on a 3s `setInterval`. Every
transport dial loop, session keepalive, and (on Android) the BLE peripheral
start hangs off that tick.

That is fine on desktop and wrong on Android, for two independent reasons that
are easy to conflate:

1. **Nobody drives the timer.** A backgrounded WebView has its JS timers
   throttled, so the interval slows or stops.
2. **Nothing is running at all.** Android freezes, and eventually kills, the
   process of an app with no foreground component. No in-process timer of any
   kind survives this.

Fixing only (1) does nothing about (2), which is the larger problem.

This was observed directly rather than reasoned about. During real-hardware
Bluetooth testing on a Pixel 6 Pro, backgrounding the Fini debug app produced
**zero log lines from our process** across a full 7,476-line logcat capture,
while the app was still listed as running. The peer's Bluetooth session then
died of missed pings. Foregrounding the app immediately restored both, and the
session authenticated on both sides seconds later.

## Decision

Two complementary mechanisms, one per problem.

### 1. A Rust-side tick keeper (`space_sync/commands.rs`)

A background task that ticks only when nothing else has within
`TICK_INTERVAL` (3s, matching the frontend's own interval).

It is deliberately a **keeper, not a replacement scheduler**. The frontend
keeps its loop, because it also refreshes UI state from each tick's result —
something a Rust loop cannot do. When the webview is driving ticks normally the
keeper stays permanently idle, so the work is never doubled.

It starts from the **first tick**, not from `.setup()`. On Android that first
JS-triggered tick is already established as the earliest point the BLE
peripheral role can safely start; starting a Rust timer earlier would
reintroduce exactly that problem from another thread.

### 2. An Android foreground service (`SyncForegroundService.kt`)

A `Service` with `foregroundServiceType="connectedDevice"` and a persistent
low-importance notification, started over the existing plain-JNI bridge in
`services::android_context`.

`connectedDevice` is the category Android defines for maintaining a link to an
external device, and from Android 14 the declared type must be backed by a
matching `FOREGROUND_SERVICE_CONNECTED_DEVICE` permission. This is the same
shape wearable companion apps use — the persistent "connected" entry a fitness
band shows in the notification shade.

`START_STICKY`, so the service is restarted if the process is reclaimed under
memory pressure. A sync daemon that stays silently dead after one low-memory
event is the failure this exists to prevent.

## Consequences

**The notification is mandatory, not a design choice.** Android requires a
foreground service to post one and shows it for as long as the service runs. It
is worth treating as a feature rather than a tax: it is honest disclosure that
Fini is holding a device connection, and the user's way to notice and stop it.
The channel is `IMPORTANCE_LOW` so it never makes a sound or peeks — it is a
status line, not an alert.

**Battery.** A permanently-running service that dials and keeps sessions open
costs power. Not yet addressed and deliberately out of scope here: the service
currently runs unconditionally once sync starts.

**No Settings switch yet.** The repo's own convention is that background-worker
features carry an explicit disable path, and this does not have one. That gap
is real and should be closed before this ships to users — see below.

## Follow-up work

- A Settings toggle to disable background sync: what stops, what data remains,
  and what re-enabling does.
- Live notification text. `SyncForegroundService.updateStatus` exists and
  replaces the shade text in place, but nothing calls it yet, so the
  notification always reads "Syncing with your devices" rather than naming the
  connected peer.
- Stopping the service when the last pairing is removed or sync is disabled.
  `stop` exists and is unused.

## Verification

Linux: `make desktop-debug-build` clean.

Android, on a physical Pixel 6 Pro:

**The service runs with the right type.** `dumpsys activity services` reports
`isForeground=true foregroundId=4211 types=0x00000010` (0x10 is
`FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE`) and a notification on channel
`fini.sync` with `ONGOING_EVENT|NO_CLEAR|FOREGROUND_SERVICE`.

**The process keeps running while backgrounded.** With the launcher in the
foreground, our process kept logging its own 60s retry loop — the direct
inverse of the silent 7,476-line capture that motivated this ADR.

**It reacts to the world while backgrounded.** Bluetooth was toggled on with
the app still backgrounded; the process noticed on its next retry and brought
the peripheral up by itself:

```
00:57:18 advertise: started, generation=2
00:57:18 serve: advertising accepted, awaiting centrals
```

**A Bluetooth session establishes and authenticates while backgrounded.** With
the launcher still foreground, the desktop dialled in and both sides
authenticated, reaching the green state (`bluetooth: configured, code=null`):

```
04:59:25 link established session=95, ATT MTU 512, fragment_budget=501
04:59:26 auth OK with 1a3ae309-... via 58:24:29:D8:0B:72
```

**The process is protected from background reclamation.** `adb shell am kill`
left the pid unchanged. That is the expected result rather than a failed test:
`am kill` only reclaims processes that are safe to kill, and a foreground
service makes this one not safe to kill. `START_STICKY` covers genuine
low-memory reclamation, which cannot be forced deterministically, so that path
remains untested.

## Out of scope: sessions do not stay up

Established sessions degrade to `ping_missed` within ~30s. This is **not** a
background-lifecycle problem — everything above happened with the app
backgrounded and the process demonstrably alive. It is a separate BLE defect:
both peers dial each other, so two links exist for one pair and the second
supersedes the first.

```
04:59:25 [central role]    link established session=95  fragment_budget=501
04:59:26 auth OK
05:00:35 [peripheral role] accepted central             session=96 fragment_budget=12
05:00:38 link lost session=96 (peripheral role)
05:00:38 notify: refusing — session 96 for ... has been superseded
```

ADR-0003's revision has both transports connect unconditionally, which is right
for Network but collides on Bluetooth, where a pair should hold one link.
Tracked separately.
