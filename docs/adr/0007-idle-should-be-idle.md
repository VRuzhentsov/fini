# 0007 — An idle pair should cost nothing

## Status

Accepted, implemented. Slices 1–3 of issue #171. The fourth — negotiating a
long BLE connection interval — needs `ble-gatt` to expose the parameter and is
not in this change.

**Not yet measured on hardware.** #171 asks for a before/after battery figure
over a multi-hour idle period on the Pixel; the phone was not attached when
this landed. The arithmetic below is wakeup counts, which is what the change
controls directly, not milliamp-hours.

## Context

Fini synced by polling, on intervals chosen when the only transport was a
network socket on mains power. Two loops drove it:

| What | Interval | Per day |
|---|---|---|
| Sync tick | 3s | ~28,800 |
| App-level ping | 15s | ~5,760 per connected transport, each direction |

Since ADR-0004 that runs inside an Android foreground service, and since
ADR-0006 over a Bluetooth link as well. For a product whose real traffic is a
handful of quest changes a day, that is tens of thousands of wakeups to deliver
perhaps five pieces of news.

The expensive thing is not the Bluetooth link. An idle BLE connection with a
long connection interval draws microamps — it is how a fitness band lasts
weeks. The expensive operations in BLE are scanning and connection setup, and
holding a connection open costs less than tearing it down and rebuilding it.
What cost us was our own polling on top: we woke the radio because a timer
said so.

## Decision

Two things move data, and neither is a timer.

**A local change wakes the drain.** `outbox::emit_sync_event_at` is the single
funnel all eighteen local-change call sites pass through, so it is the one
place that can say "there is something to send". It now raises a `Notify`, and
the keeper selects on that against the backstop interval. The signal is raised
*after* the insert: a waiter woken by it re-reads the outbox from SQLite, so
signalling first would race it into finding nothing.

> **Correction (review of this PR).** As first written this held on Android
> only, and the section below was wrong in two ways.
>
> The keeper was `#[cfg(target_os = "android")]`, and it is the only thing
> that waits on that `Notify` — so on desktop the signal was raised into a
> void and a local edit waited for the frontend's cadence, which this same
> change had just slowed from 3s to 30s. The keeper now runs everywhere;
> only its *periodic* arm stays Android-only, which is what the SQLite lock
> contention documented at its call site was actually about.
>
> Inbound frames raised no signal at all. `handle_inbound` queues an
> envelope and returns, so a peer's edit also waited for the backstop —
> "pushed" was true of the queue, not of the database or the UI. Both
> `SyncEvent` and `SpaceMappingUpdate` now raise it, and the latter also
> raises `space-sync://changed`, because mapping updates are drained by the
> frontend rather than by a tick.
>
> The `Notify` is now `notify_one`, not `notify_waiters`: a signal raised
> while the keeper is mid-tick has to survive until it next waits. The
> original reasoning — that the running tick would read those rows anyway —
> only holds if the write lands before that tick's read, and nothing makes
> that true for a frame arriving on another task.

**A remote change pushes to the UI.** When a tick applies a peer's event, a
broadcast fires; `lib.rs` forwards it to the webview as `space-sync://changed`,
the same shape ADR-0003 Phase 2 already uses for session lifecycle. The event
carries no payload — what changed is in SQLite, and the frontend re-reads it
through the commands it already uses. A second source of truth would buy
nothing.

With data movement event-driven, the periodic work is only a backstop:
re-arming dial loops for a peer that is *not* connected, draining incoming
space-sync ends, and covering a dropped notification. `TICK_INTERVAL` goes
3s → 30s, and the frontend's own loop ticks every sixth heartbeat rather than
continuously.

**The ping goes 15s → 2 minutes.** What makes that safe is that a dropped link
was never detected by the ping: `run_session`'s loop breaks the moment its
receive path errors or the peer closes, and calls `release_session`, which
raises `LinkEvent::SessionEnded`. That is the radio telling us, and it is
faster and more trustworthy than counting missed pings. What is left for the
ping is the case the transport cannot see — a peer whose link is up but whose
app has stopped answering.

## The trap this had to avoid

`frontend_is_driving()` decides whether Bluetooth scans on its foreground
cadence (30s) or its frugal background one (60s), and it answered that
question by observing *sync ticks arriving* — the webview only ticks while it
is alive, so a recent tick meant someone was watching.

Making sync event-driven removes those ticks, and would have taken that signal
with them: discovery would have quietly dropped to the background period while
the user sat watching a row, waiting for it to turn green. Nothing would have
failed; it would just have been slower, for a reason nothing in the diff
mentioned.

So the two meanings, previously conflated in one tick, are now separate:
`space_sync_note_foreground` carries "someone is watching" and does no work at
all — no database, no dial loops, no outbox — while data movement is driven by
the events above. The heartbeat stays frequent (5s) precisely because it is now
cheap enough for that to be irrelevant.

## Consequences

Wakeups that carry no news, per day, for one connected pair:

| | Before | After |
|---|---|---|
| Sync tick (backend keeper) | ~28,800 | ~2,880 |
| App ping, per transport per direction | ~5,760 | ~720 |
| Foreground heartbeat | — | only while the app is open, and does nothing |

Latency moves the other way: a local edit no longer waits up to 3s for a tick,
and a remote one no longer waits for the peer's next poll.

**What gets slower.** `TransportAckState`'s three-miss decay to `PingMissed`
now takes ~6 minutes rather than ~45s. That governs only a live-but-wedged
peer; every ordinary disconnect still turns the row over immediately, via
`SessionEnded`.

## Verification

Done: `cargo test --lib` 294/294; `vue-tsc` clean; frontend suites 149/149.

Owed, and listed in #171's acceptance criteria:

- A multi-hour idle battery measurement on the Pixel, before and after.
- On-device confirmation that a quest changed on one device appears on the
  other without waiting for a poll interval.
- On-device confirmation that a link dropped at the radio level still turns
  the row over as fast as before.
