use diesel::prelude::*;
use tauri::State;

use crate::models::{CreateReminderInput, Reminder};
use crate::schema::reminders;
use crate::services::db::DbState;

#[tauri::command]
pub fn get_reminders(state: State<DbState>, quest_id: String) -> Result<Vec<Reminder>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    reminders::table
        .filter(reminders::quest_id.eq(&quest_id))
        .select(Reminder::as_select())
        .load(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_reminder(
    state: State<DbState>,
    input: CreateReminderInput,
) -> Result<Reminder, String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::insert_into(reminders::table)
        .values(&input)
        .returning(Reminder::as_returning())
        .get_result(&mut *conn)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_reminder(state: State<DbState>, id: String) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();
    diesel::delete(reminders::table.find(&id))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}
