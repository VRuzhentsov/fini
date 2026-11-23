use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::models::{CreateSpaceInput, Space, UpdateSpaceInput};
use crate::schema::spaces;

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
