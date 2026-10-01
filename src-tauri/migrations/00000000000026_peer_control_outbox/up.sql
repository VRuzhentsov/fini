-- ADR-0008 D14: the frames one device sends another on its own -- a channel
-- unlinked, a space no longer synced -- must reach the peer even across a
-- restart or a dropped link. Sync events do not need this: what to send is
-- worked out again from the two devices' state on every exchange.
--
-- `subject` is the channel kind or the space id the frame is about, so a
-- later act on the same subject (the channel set up again, the space
-- requested again) can drop a frame that no longer holds.
CREATE TABLE peer_control_outbox (
    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    peer_device_id TEXT NOT NULL REFERENCES paired_devices(peer_device_id) ON DELETE CASCADE,
    frame_type TEXT NOT NULL,
    subject TEXT NOT NULL,
    frame TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX peer_control_outbox_peer ON peer_control_outbox (peer_device_id);
