# DevicesSettingsSection

Dynamic Settings overview section for paired devices, incoming pair requests, and the entry point to pairing.

Each device row carries a plain-language summary line built from whatever channel state is already cached — the presence loop refreshes it for every presenced peer, so the summary costs no extra calls and degrades to a bare "Not connected" rather than inventing a reason.

Pairing itself is [[PairDeviceDialog]], opened from the "Add device" row. There is no route for it — see [[SettingsView]]. The Settings search can also ask for it, through the `pairRequests` prop: a counter rather than a flag, so asking twice works. A flag would already be true the second time and nothing would happen.
