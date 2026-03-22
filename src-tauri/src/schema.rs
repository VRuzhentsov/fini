diesel::table! {
    spaces (id) {
        id         -> Text,
        name       -> Text,
        item_order -> BigInt,
        created_at -> Text,
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
        created_at  -> Text,
        updated_at  -> Text,
    }
}

diesel::joinable!(quests -> spaces (space_id));
diesel::allow_tables_to_appear_in_same_query!(spaces, quests);
