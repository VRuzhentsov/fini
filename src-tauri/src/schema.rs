diesel::table! {
    spaces (id) {
        id         -> Text,
        name       -> Text,
        item_order -> BigInt,
        created_at -> Text,
    }
}

diesel::table! {
    settings (key) {
        key   -> Text,
        value -> Text,
    }
}

diesel::table! {
    quest_series (id) {
        id          -> Text,
        space_id    -> Text,
        title       -> Text,
        description -> Nullable<Text>,
        repeat_rule -> Text,
        priority    -> BigInt,
        energy      -> Text,
        active      -> Bool,
        created_at  -> Text,
        updated_at  -> Text,
    }
}

diesel::table! {
    quests (id) {
        id          -> Text,
        space_id    -> Text,
        title       -> Text,
        description -> Nullable<Text>,
        status      -> Text,
        energy      -> Text,
        priority    -> BigInt,
        pinned      -> Bool,
        due         -> Nullable<Text>,
        due_time    -> Nullable<Text>,
        repeat_rule -> Nullable<Text>,
        completed_at -> Nullable<Text>,
        order_rank -> Double,
        created_at  -> Text,
        updated_at  -> Text,
        series_id   -> Nullable<Text>,
        period_key  -> Nullable<Text>,
    }
}

diesel::table! {
    reminders (id) {
        id                       -> Text,
        quest_id                 -> Text,
        #[sql_name = "type"]
        kind                     -> Text,
        mm_offset                -> Nullable<BigInt>,
        due_at_utc               -> Nullable<Text>,
        created_at               -> Text,
        scheduled_notification_id -> Nullable<Text>,
    }
}

diesel::table! {
    series_reminder_templates (id) {
        id         -> Text,
        series_id  -> Text,
        kind       -> Text,
        mm_offset  -> Nullable<BigInt>,
        created_at -> Text,
    }
}

diesel::table! {
    paired_devices (peer_device_id) {
        peer_device_id -> Text,
        display_name   -> Text,
        paired_at      -> Text,
        last_seen_at   -> Nullable<Text>,
        pair_state     -> Text,
    }
}

diesel::table! {
    pair_space_mappings (peer_device_id, space_id) {
        peer_device_id -> Text,
        space_id       -> Text,
        enabled_at     -> Text,
        last_synced_at -> Nullable<Text>,
    }
}

diesel::table! {
    focus_history (id) {
        id         -> Text,
        quest_id   -> Text,
        space_id   -> Text,
        trigger    -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    sync_outbox (event_id) {
        event_id         -> Text,
        correlation_id   -> Text,
        origin_device_id -> Text,
        entity_type      -> Text,
        entity_id        -> Text,
        space_id         -> Text,
        op_type          -> Text,
        payload          -> Nullable<Text>,
        updated_at       -> Text,
        created_at       -> Text,
    }
}

diesel::table! {
    sync_acks (peer_device_id, event_id) {
        peer_device_id -> Text,
        event_id       -> Text,
        acked_at       -> Text,
    }
}

diesel::table! {
    sync_seen (event_id) {
        event_id    -> Text,
        received_at -> Text,
    }
}

diesel::table! {
    tombstones (entity_type, entity_id) {
        entity_type -> Text,
        entity_id   -> Text,
        space_id    -> Text,
        deleted_at  -> Text,
    }
}

diesel::joinable!(quests -> spaces (space_id));
diesel::joinable!(quests -> quest_series (series_id));
diesel::joinable!(quest_series -> spaces (space_id));
diesel::joinable!(reminders -> quests (quest_id));
diesel::joinable!(series_reminder_templates -> quest_series (series_id));
diesel::joinable!(pair_space_mappings -> paired_devices (peer_device_id));
diesel::joinable!(pair_space_mappings -> spaces (space_id));
diesel::joinable!(focus_history -> quests (quest_id));

diesel::allow_tables_to_appear_in_same_query!(
    spaces,
    settings,
    quests,
    quest_series,
    reminders,
    series_reminder_templates,
    paired_devices,
    pair_space_mappings,
    focus_history,
    sync_outbox,
    sync_acks,
    sync_seen,
    tombstones
);
