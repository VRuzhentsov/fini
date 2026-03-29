use diesel::prelude::*;
use tauri::State;

use crate::models::{CreateSpaceInput, Space, UpdateSpaceInput};
use crate::schema::spaces;
use crate::services::db::DbState;

#[tauri::command]
pub fn get_spaces(state: State<DbState>) -> Result<Vec<Space>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    spaces::table
        .select(Space::as_select())
        .order(spaces::item_order.asc())
        .load(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_space(state: State<DbState>, input: CreateSpaceInput) -> Result<Space, String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::insert_into(spaces::table)
        .values(&input)
        .returning(Space::as_returning())
        .get_result(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_space(
    state: State<DbState>,
    id: String,
    input: UpdateSpaceInput,
) -> Result<Space, String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::update(spaces::table.find(&id))
        .set(&input)
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    spaces::table
        .find(&id)
        .select(Space::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_space(state: State<DbState>, id: String) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::delete(spaces::table.find(&id))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}
