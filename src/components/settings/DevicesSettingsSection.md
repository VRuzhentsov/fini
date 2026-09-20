# DevicesSettingsSection

Dynamic Settings overview section for paired devices, incoming pair requests, and the entry point to pairing.

Each device row carries a plain-language summary line built from whatever channel state is already cached — the presence loop refreshes it for every presenced peer, so the summary costs no extra calls and degrades to a bare "Not connected" rather than inventing a reason.

Pairing itself is [[PairDeviceDialog]], opened from the "Add device" row. `/settings/add-device` is no longer a page but still opens that dialog, because the Settings search lists it as a destination and deep links to it already exist.
