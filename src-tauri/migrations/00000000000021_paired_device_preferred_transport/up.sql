-- ADR-0003 Phase 3: a manually-pinned transport preference for this pair.
-- NULL means "no manual preference, pure automatic network-first
-- selection" -- every existing row's default, unchanged behavior for
-- anyone who never uses the new manual-switch UI. Values are
-- "network"/"bluetooth" (device_connection::transport::TransportKind's
-- own snake_case serde form), not the finer-grained TcpWs/Sim/Bluetooth
-- services::transport::TransportKind a live session reports.
ALTER TABLE paired_devices
ADD COLUMN preferred_transport TEXT NULL;

-- When `preferred_transport` was last set (locally, or adopted from a
-- peer's PeerFrame::SwitchTransport) -- the last-writer-wins timestamp for
-- resolving two nearly-simultaneous switches on both sides of a pair. NULL
-- alongside a NULL preferred_transport for every pre-existing row.
ALTER TABLE paired_devices
ADD COLUMN preferred_transport_set_at TEXT NULL;
