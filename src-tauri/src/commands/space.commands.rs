#[cfg(any(feature = "ui-plane", test))]
use tauri::State;

#[cfg(any(feature = "ui-plane", test))]
use crate::models::{CreateSpaceInput, Space, UpdateSpaceInput};
#[cfg(any(feature = "ui-plane", test))]
use crate::repositories::{Repository, SpaceRepository};
#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::DbPool;

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn get_spaces(state: State<DbPool>) -> Result<Vec<Space>, String> {
    let mut conn = state.get()?;
    SpaceRepository::new(&mut conn).list()
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn create_space(state: State<DbPool>, input: CreateSpaceInput) -> Result<Space, String> {
    let mut conn = state.get()?;
    SpaceRepository::new(&mut conn).create(input)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn update_space(
    state: State<DbPool>,
    id: String,
    input: UpdateSpaceInput,
) -> Result<Space, String> {
    let mut conn = state.get()?;
    SpaceRepository::new(&mut conn).update(&id, input)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn delete_space(state: State<DbPool>, id: String) -> Result<(), String> {
    let mut conn = state.get()?;
    SpaceRepository::new(&mut conn).delete(&id)
}
