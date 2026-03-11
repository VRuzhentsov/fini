diesel::table! {
    spaces (id) {
        id         -> BigInt,
        name       -> Text,
        item_order -> BigInt,
        created_at -> Text,
    }
}

diesel::table! {
    quests (id) {
        id              -> BigInt,
        space_id        -> Nullable<BigInt>,
        title           -> Text,
        description     -> Nullable<Text>,
        status          -> Text,
        energy_required -> Nullable<BigInt>,
        priority        -> BigInt,
        pinned          -> Bool,
        due             -> Nullable<Text>,
        completed_at    -> Nullable<Text>,
        created_at      -> Text,
        updated_at      -> Text,
    }
}

diesel::joinable!(quests -> spaces (space_id));
diesel::allow_tables_to_appear_in_same_query!(spaces, quests);
