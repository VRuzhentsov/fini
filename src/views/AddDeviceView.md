# AddDeviceView

Route: `/settings/add-device`. Parent: [[SettingsView]].

## Concept

Dedicated pairing mode view for discovering and pairing local devices.

Only devices currently in add-device mode are shown as pairing candidates.

## Behavior

- Entering this view enables add-device mode for local device
- Discovery refresh cadence: every 5s
- Candidate list:
  - newest seen first
  - deduplicated by `device_id`
  - excludes already paired devices
- Selecting a device starts pairing request flow

## Pairing flow rules

- 6-digit passcode is mandatory
- Sender selection:
  - first click timestamp wins
  - tie-breaker is lower `device_id`
- Receiver gets incoming request sheet
- Sender sees code only after receiver accepts
- 3 wrong attempts per remote device -> 60s cooldown
- Pending request auto-expires in 60s
- Leaving this view cancels all pending requests immediately

## Notes

- Code expiry is tied to view lifecycle (no separate timer while view is open)
- Transport encryption is deferred beyond MVP.1 (see [[spec/DeviceSync]])
