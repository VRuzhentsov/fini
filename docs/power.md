# Power

How much battery Fini may use on a phone, why, and how we measure it.

Written in STE (`docs/naming.md`). Every Fini feature that runs in the
background must keep to this budget. A change that can increase power use
adds a row to "Measurements" before it merges.

## Requirements from Google Play

Google Play shows battery use to users and removes apps that use too much
from its recommendations.

| Rule | Limit | Status |
|---|---|---|
| Excessive partial wake locks | An app holds a partial wake lock that is not exempt for 2 hours or more on average while the screen is off, in more than 5 % of user sessions over the last 28 days | Core vital for all apps. Enforcement started on 1 March 2026: a warning on the store listing, and no place in recommendations |
| Excessive wakeups, stuck partial wake locks, excessive background Wi-Fi scans, excessive background network use | No limit published | Other vitals, not core |

Exempt wake locks: wake locks that the system holds and that give a clear
benefit to the user, for example audio playback, location access, and data
transfer that the user started.

Google's guidance for Bluetooth:

- Pair devices with the companion device API (`CompanionDeviceManager`), not
  with a manual wake lock.
- For Bluetooth in the background, follow the Android guide "Communicate in
  the background" for BLE.
- Use WorkManager for work that can wait.
- If a manual wake lock is necessary, hold it only while data moves.

Sources:
[Android vitals](https://developer.android.com/topic/performance/vitals),
[Battery technical quality enforcement](https://developer.android.com/blog/posts/battery-technical-quality-enforcement-is-here-how-to-optimize-common-wake-lock-use-cases),
[Raising the bar on battery performance](https://android-developers.googleblog.com/2025/11/raising-bar-on-battery-performance.html).

## Fini's budget

Proposed on 2026-10-13. Confirm or change each line before it becomes a
rule.

| Item | Limit |
|---|---|
| Partial wake locks with the screen off | None held by Fini code. Bluetooth links are held by the Android Bluetooth service, not by a Fini wake lock |
| One held Bluetooth link to a paired device | 10 mAh or less per day on the reference phone |
| Bluetooth scans with the screen off | Only scans with a hardware filter on Fini's service UUID. No scan loop in Fini code |
| Bluetooth scans without a filter | Only while a person has a Fini screen open that needs them (Add Device, adding a channel) |
| Network channel | One kept-alive iroh connection per paired device. No polling |

Reference phone: Pixel 6 Pro, Android beta, the user's daily phone with
other Bluetooth devices (headphones, keyboard) connected.

## How each feature keeps to the budget

| Feature | How |
|---|---|
| Bluetooth "green" for a paired device | One held BLE connection with the low-power connection priority (`ConnectionPriority::LowPower`). High priority only while an exchange sends data |
| Bluetooth reconnect after a lost link | Android background scan with a filter on Fini's service UUID; the Bluetooth chip does the filter, so the app wakes only when the device is near |
| Add Device over Bluetooth | The scan runs only while the Add Device dialog is open and stops when it closes |
| Network "green" | A kept-alive iroh connection; iroh's mDNS lookup gives the address |
| Sync | SpaceSync sends queued changes only over a green channel. It has no timer and no retry loop |

## How we measure

On the reference phone, with no USB cable (Android counts battery use only on
battery), `adb` over Wi-Fi, the screen off, and the same other Bluetooth
devices connected for every run:

1. `adb shell dumpsys batterystats --reset`
2. Start the case (for example, hold a link), then switch the screen off.
3. Wait 10 minutes.
4. `adb shell dumpsys batterystats > run.txt`
5. Read:
   - `Estimated power use (mAh)` → `Global` → `bluetooth:` (Bluetooth energy
     for the whole phone),
   - `Bluetooth Idle / Rx / Tx time` (time of the Bluetooth chip in each
     state),
   - the app's own `UID` line.
6. Compare with a run without the case (baseline) made in the same session.

For wake locks, use the app's `Wake lock` lines in the same output, or a
Perfetto trace.

These are Android's own estimates, not a power meter. One run is one
sample.

## Measurements

| Date | Case | Bluetooth energy per 10 min | Link | Notes |
|---|---|---|---|---|
| 2026-10-13 | Baseline: no Fini link | 0.166 mAh | — | Android's own background scan (UID 1000) ran the full 10 min in all three runs |
| 2026-10-13 | One held BLE link, `LowPower` | 0.215 mAh | held 590.8 of 600 s, 0 drops | About +7 mAh per day (0.2 % of 3236 mAh). In an earlier run the link stayed up for 15 min while Android froze Fini in the background |
| 2026-10-13 | One held BLE link, `High` | 0.255 mAh | held 588.4 of 600 s, 0 drops | About +13 mAh per day |

In these three runs the phone connected to the desktop as the central.
The desktop's radio (Intel AX210) also served Wi-Fi and the user's
Bluetooth headphones.
