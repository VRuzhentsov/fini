use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::pair_space_mappings;

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = pair_space_mappings)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct PairSpaceMapping {
    pub peer_device_id: String,
    pub space_id: String,
    pub enabled_at: String,
    pub last_synced_at: Option<String>,
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = pair_space_mappings)]
pub struct CreatePairSpaceMappingInput {
    pub peer_device_id: String,
    pub space_id: String,
    pub enabled_at: String,
    pub last_synced_at: Option<String>,
}
