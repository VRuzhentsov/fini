CREATE TABLE pair_space_mappings__old (
    peer_device_id TEXT NOT NULL REFERENCES paired_devices(peer_device_id) ON DELETE CASCADE,
    space_id       TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    enabled_at     TEXT NOT NULL,
    PRIMARY KEY (peer_device_id, space_id)
);

INSERT INTO pair_space_mappings__old (peer_device_id, space_id, enabled_at)
SELECT peer_device_id, space_id, enabled_at
FROM pair_space_mappings;

DROP TABLE pair_space_mappings;

ALTER TABLE pair_space_mappings__old RENAME TO pair_space_mappings;
