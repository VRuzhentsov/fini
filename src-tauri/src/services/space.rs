use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
#[cfg(any(feature = "ui-plane", test))]
use tauri::State;

use crate::models::{CreateSpaceInput, Space, UpdateSpaceInput};
use crate::schema::spaces;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::AppDbConnection;

pub struct SpaceRepository<'a> {
    conn: &'a mut SqliteConnection,
}

impl<'a> SpaceRepository<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn list(&mut self) -> Result<Vec<Space>, String> {
        spaces::table
            .select(Space::as_select())
            .order(spaces::item_order.asc())
            .load(self.conn)
            .map_err(|e| e.to_string())
    }

    pub fn create(&mut self, input: CreateSpaceInput) -> Result<Space, String> {
        diesel::insert_into(spaces::table)
            .values(&input)
            .returning(Space::as_returning())
            .get_result(self.conn)
            .map_err(|e| e.to_string())
    }

    pub fn update(&mut self, id: &str, input: UpdateSpaceInput) -> Result<Space, String> {
        diesel::update(spaces::table.find(id))
            .set(&input)
            .execute(self.conn)
            .map_err(|e| e.to_string())?;
        spaces::table
            .find(id)
            .select(Space::as_select())
            .first(self.conn)
            .map_err(|e| e.to_string())
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        diesel::delete(spaces::table.find(id))
            .execute(self.conn)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn get_spaces(state: State<AppDbConnection>) -> Result<Vec<Space>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    SpaceRepository::new(&mut conn).list()
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn create_space(
    state: State<AppDbConnection>,
    input: CreateSpaceInput,
) -> Result<Space, String> {
    let mut conn = state.inner().0.lock().unwrap();
    SpaceRepository::new(&mut conn).create(input)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn update_space(
    state: State<AppDbConnection>,
    id: String,
    input: UpdateSpaceInput,
) -> Result<Space, String> {
    let mut conn = state.inner().0.lock().unwrap();
    SpaceRepository::new(&mut conn).update(&id, input)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn delete_space(state: State<AppDbConnection>, id: String) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();
    SpaceRepository::new(&mut conn).delete(&id)
}
