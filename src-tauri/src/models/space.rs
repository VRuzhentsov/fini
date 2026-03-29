use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::spaces;

#[derive(Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = spaces)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Space {
    pub id: String,
    pub name: String,
    pub item_order: i64,
    pub created_at: String,
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = spaces)]
pub struct CreateSpaceInput {
    pub name: String,
    pub item_order: i64,
}

#[derive(Deserialize, AsChangeset)]
#[diesel(table_name = spaces)]
pub struct UpdateSpaceInput {
    pub name: Option<String>,
    pub item_order: Option<i64>,
}
