use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::settings;

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = settings)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Setting {
    pub key: String,
    pub value: String,
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = settings)]
pub struct UpsertSettingInput {
    pub key: String,
    pub value: String,
}
