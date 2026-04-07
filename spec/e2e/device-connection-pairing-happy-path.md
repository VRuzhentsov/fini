---
title: Device Connection Pairing Happy Path
type: e2e
area: device-connection
status: draft
---

# Device Connection Pairing Happy Path

## Goal

Validate that two unpaired devices can discover each other, complete passcode pairing, and appear as paired peers in Settings.

## Preconditions

- Device A and Device B are on the same LAN.
- Fini is installed and launched on both devices.
- Devices are not currently paired with each other.
- Both devices have valid local identity (`device_id`, `hostname`).

## Test Steps

1. On both devices, open `Settings -> Add device`.
2. Wait for discovery refresh and confirm each device appears in the other candidate list.
3. On Device A, tap Device B to send a pairing request.
4. On Device B, accept the incoming request.
5. On Device A, read the shown 6-digit pairing code.
6. On Device B, enter the same 6-digit code and submit.
7. Wait for pairing success state and return both devices to `Settings`.
8. Open `Settings -> Devices` on both devices.
9. Tap the peer row on each side to open `Settings -> Device/:id`.

## Assertions

- Pairing completes within request timeout window.
- Both devices show each other in `Settings -> Devices`.
- Peer row opens device detail page on both sides.
- `Add device` list no longer shows the already paired peer.
- Device detail shows presence metadata (`online/last seen`) updating over time.

## Failure Signals

- Candidate discovery list does not populate while both are in add mode.
- Pair request/accept flow stalls before code entry.
- Code validation incorrectly accepts wrong code or blocks correct code.
- Pair appears only on one device.
- Paired device can still be selected as an add-mode candidate.

## Evidence Artifacts

- Before screenshots from both devices in add mode.
- Pair code screen screenshot on sender + code entry screenshot on receiver.
- After screenshots of `Settings -> Devices` on both devices.
- Optional logs around `device_connection_pair_*` command flow.
