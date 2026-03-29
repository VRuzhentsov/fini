use super::types::SyncEventEnvelope;

/// Determines whether incoming event wins over existing event using the locked conflict policy:
/// 1. newer `updated_at` wins
/// 2. tie-break: lexicographically lower `origin_device_id` wins
/// 3. final tie-break: lexicographically lower `event_id` wins
pub fn incoming_wins(incoming: &SyncEventEnvelope, existing: &SyncEventEnvelope) -> bool {
    match incoming.updated_at.cmp(&existing.updated_at) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => {
            match incoming.origin_device_id.cmp(&existing.origin_device_id) {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Greater => false,
                std::cmp::Ordering::Equal => incoming.event_id < existing.event_id,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(event_id: &str, origin_device_id: &str, updated_at: &str) -> SyncEventEnvelope {
        SyncEventEnvelope {
            event_id: event_id.to_string(),
            correlation_id: "corr-1".to_string(),
            origin_device_id: origin_device_id.to_string(),
            entity_type: "quest".to_string(),
            entity_id: "q1".to_string(),
            space_id: "1".to_string(),
            op_type: "upsert".to_string(),
            payload: None,
            updated_at: updated_at.to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn newer_updated_at_wins() {
        let incoming = make_event("e1", "device-a", "2026-03-02T00:00:00Z");
        let existing = make_event("e2", "device-b", "2026-03-01T00:00:00Z");
        assert!(incoming_wins(&incoming, &existing));
    }

    #[test]
    fn older_updated_at_loses() {
        let incoming = make_event("e1", "device-a", "2026-03-01T00:00:00Z");
        let existing = make_event("e2", "device-b", "2026-03-02T00:00:00Z");
        assert!(!incoming_wins(&incoming, &existing));
    }

    #[test]
    fn same_updated_at_lower_device_id_wins() {
        let incoming = make_event("e1", "aaa", "2026-03-01T00:00:00Z");
        let existing = make_event("e2", "bbb", "2026-03-01T00:00:00Z");
        assert!(incoming_wins(&incoming, &existing));
    }

    #[test]
    fn same_updated_at_higher_device_id_loses() {
        let incoming = make_event("e1", "bbb", "2026-03-01T00:00:00Z");
        let existing = make_event("e2", "aaa", "2026-03-01T00:00:00Z");
        assert!(!incoming_wins(&incoming, &existing));
    }

    #[test]
    fn same_updated_at_same_device_lower_event_id_wins() {
        let incoming = make_event("aaa", "device-a", "2026-03-01T00:00:00Z");
        let existing = make_event("bbb", "device-a", "2026-03-01T00:00:00Z");
        assert!(incoming_wins(&incoming, &existing));
    }

    #[test]
    fn same_updated_at_same_device_higher_event_id_loses() {
        let incoming = make_event("bbb", "device-a", "2026-03-01T00:00:00Z");
        let existing = make_event("aaa", "device-a", "2026-03-01T00:00:00Z");
        assert!(!incoming_wins(&incoming, &existing));
    }
}
