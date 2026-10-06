-- ADR-0009 D8: the peer's iroh key (its EndpointId, hex), pinned when the
-- pair is made and checked on every connection. NULL for a pair made before
-- keys existed; such a pair is paired again.
ALTER TABLE paired_devices ADD COLUMN endpoint_id TEXT;
