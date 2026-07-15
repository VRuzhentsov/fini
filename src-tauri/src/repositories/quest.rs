use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use chrono::Utc;

use crate::models::{
    clamp_order_rank, CreateQuestInput, CreateSeriesInput, Quest, QuestFieldPatch, QuestSeries,
    QuestUpdatePatch, Space, UpdateQuestInput,
};
use crate::schema::{quest_series, quests, spaces};
use crate::services::db::utc_now;

/// Diesel persistence boundary for quest records. Lifecycle policies remain in QuestService.
pub struct QuestRepository<'a> {
    pub(crate) conn: &'a mut SqliteConnection,
}

impl<'a> QuestRepository<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn load_all(&mut self) -> Result<Vec<Quest>, String> {
        crate::schema::quests::table
            .select(Quest::as_select())
            .load(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn get(&mut self, id: &str) -> Result<Quest, String> {
        crate::schema::quests::table
            .find(id)
            .select(Quest::as_select())
            .first(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn load_quest_and_space(&mut self, quest_id: &str) -> Result<(Quest, Space), String> {
        let quest = self.get(quest_id)?;
        let space = spaces::table
            .find(&quest.space_id)
            .select(Space::as_select())
            .first(self.conn)
            .map_err(|error| error.to_string())?;
        Ok((quest, space))
    }

    pub fn get_active(&mut self, id: &str) -> Result<Option<Quest>, String> {
        quests::table
            .find(id)
            .filter(quests::status.eq("active"))
            .select(Quest::as_select())
            .first(self.conn)
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn is_series_active(&mut self, id: &str) -> Result<bool, String> {
        quest_series::table
            .find(id)
            .select(quest_series::active)
            .first(self.conn)
            .optional()
            .map(|active| active.unwrap_or(true))
            .map_err(|error| error.to_string())
    }

    pub fn create_series(&mut self, input: &CreateSeriesInput) -> Result<QuestSeries, String> {
        diesel::insert_into(quest_series::table)
            .values(input)
            .returning(QuestSeries::as_returning())
            .get_result(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn link_to_series(
        &mut self,
        quest_id: &str,
        series_id: &str,
        period_key: &str,
    ) -> Result<(), String> {
        diesel::update(quests::table.find(quest_id))
            .set((
                quests::series_id.eq(series_id),
                quests::period_key.eq(period_key),
            ))
            .execute(self.conn)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn count_completed_series_occurrences(&mut self, series_id: &str) -> Result<i64, String> {
        quests::table
            .filter(quests::series_id.eq(series_id))
            .filter(quests::status.ne("active"))
            .count()
            .get_result(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn deactivate_series(&mut self, series_id: &str) -> Result<(), String> {
        diesel::update(quest_series::table.find(series_id))
            .set(quest_series::active.eq(false))
            .execute(self.conn)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn series_has_occurrence(
        &mut self,
        series_id: &str,
        period_key: &str,
    ) -> Result<bool, String> {
        quests::table
            .filter(quests::series_id.eq(series_id))
            .filter(quests::period_key.eq(period_key))
            .count()
            .get_result::<i64>(self.conn)
            .map(|count| count > 0)
            .map_err(|error| error.to_string())
    }

    pub fn create_next_occurrence(
        &mut self,
        quest: &Quest,
        series_id: &str,
        period_key: &str,
    ) -> Result<Option<Quest>, String> {
        let max_rank = quests::table
            .select(diesel::dsl::max(quests::order_rank))
            .first::<Option<f64>>(self.conn)
            .map_err(|error| error.to_string())?
            .unwrap_or(0.0);
        let now = utc_now();
        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq(&quest.space_id),
                quests::title.eq(&quest.title),
                quests::description.eq(quest.description.as_deref()),
                quests::status.eq("active"),
                quests::energy.eq(&quest.energy),
                quests::priority.eq(quest.priority),
                quests::due.eq(period_key),
                quests::due_time.eq(quest.due_time.as_deref()),
                quests::repeat_rule.eq(quest.repeat_rule.as_deref()),
                quests::order_rank.eq(max_rank + 1.0),
                quests::series_id.eq(series_id),
                quests::period_key.eq(period_key),
                quests::created_at.eq(&now),
                quests::updated_at.eq(&now),
            ))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        quests::table
            .filter(quests::series_id.eq(series_id))
            .filter(quests::period_key.eq(period_key))
            .select(Quest::as_select())
            .first(self.conn)
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn touch(&mut self, id: &str) -> Result<(), String> {
        diesel::update(quests::table.find(id))
            .set(quests::updated_at.eq(utc_now()))
            .execute(self.conn)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn increment_focus_enter_count(&mut self, id: &str) -> Result<Quest, String> {
        diesel::update(quests::table.find(id))
            .set(quests::focus_enter_count.eq(quests::focus_enter_count + 1))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        self.get(id)
    }

    pub fn series_space_id(&mut self, series_id: &str) -> Result<String, String> {
        quest_series::table
            .find(series_id)
            .select(quest_series::space_id)
            .first(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn list_for_series(&mut self, series_id: &str) -> Result<Vec<Quest>, String> {
        quests::table
            .filter(quests::series_id.eq(series_id))
            .select(Quest::as_select())
            .load(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn delete_series_quest(&mut self, id: &str) -> Result<(), String> {
        diesel::delete(quests::table.find(id))
            .execute(self.conn)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn delete_series(&mut self, id: &str) -> Result<(), String> {
        diesel::delete(quest_series::table.find(id))
            .execute(self.conn)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn create(&mut self, input: CreateQuestInput) -> Result<Quest, String> {
        let max_rank = quests::table
            .select(diesel::dsl::max(quests::order_rank))
            .first::<Option<f64>>(self.conn)
            .map_err(|error| error.to_string())?
            .unwrap_or(0.0);
        let input = CreateQuestInput {
            order_rank: Some(clamp_order_rank(input.order_rank.unwrap_or(max_rank + 1.0))),
            ..input
        };
        diesel::insert_into(quests::table)
            .values(&input)
            .returning(Quest::as_returning())
            .get_result(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn create_series_and_quest(
        &mut self,
        input: CreateQuestInput,
    ) -> Result<(Quest, QuestSeries), String> {
        let max_rank = quests::table
            .select(diesel::dsl::max(quests::order_rank))
            .first::<Option<f64>>(self.conn)
            .map_err(|error| error.to_string())?
            .unwrap_or(0.0);
        let repeat_rule = input
            .repeat_rule
            .clone()
            .ok_or_else(|| "repeat rule is required".to_string())?;
        let series = diesel::insert_into(quest_series::table)
            .values(&CreateSeriesInput {
                space_id: input.space_id.clone(),
                title: input.title.clone(),
                description: input.description.clone(),
                repeat_rule,
                priority: input.priority,
                energy: input.energy.clone(),
            })
            .returning(QuestSeries::as_returning())
            .get_result(self.conn)
            .map_err(|error| error.to_string())?;
        let period_key = input
            .due
            .clone()
            .unwrap_or_else(|| Utc::now().format("%Y-%m-%d").to_string());
        let now = utc_now();
        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq(&input.space_id),
                quests::title.eq(&input.title),
                quests::description.eq(&input.description),
                quests::status.eq("active"),
                quests::energy.eq(&input.energy),
                quests::priority.eq(input.priority),
                quests::due.eq(&input.due),
                quests::due_time.eq(&input.due_time),
                quests::repeat_rule.eq(&input.repeat_rule),
                quests::order_rank.eq(clamp_order_rank(input.order_rank.unwrap_or(max_rank + 1.0))),
                quests::series_id.eq(&series.id),
                quests::period_key.eq(&period_key),
                quests::created_at.eq(&now),
                quests::updated_at.eq(&now),
            ))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        let quest = quests::table
            .filter(quests::series_id.eq(&series.id))
            .filter(quests::period_key.eq(&period_key))
            .select(Quest::as_select())
            .first(self.conn)
            .map_err(|error| error.to_string())?;
        Ok((quest, series))
    }

    pub fn update(
        &mut self,
        id: &str,
        mut input: UpdateQuestInput,
    ) -> Result<(Quest, Quest), String> {
        let previous = self.get(id)?;
        if let Some(rank) = input.order_rank {
            input.order_rank = Some(clamp_order_rank(rank));
        }
        if let (Some(series_id), Some(new_due)) =
            (previous.series_id.as_deref(), input.due.as_deref())
        {
            let conflicts = quests::table
                .filter(quests::series_id.eq(series_id))
                .filter(quests::period_key.eq(new_due))
                .filter(quests::id.ne(id))
                .count()
                .get_result::<i64>(self.conn)
                .map_err(|error| error.to_string())?;
            if conflicts > 0 {
                return Err("occurrence for this date already exists in the series".to_string());
            }
        }
        let status = input.status.clone();
        diesel::update(quests::table.find(id))
            .set((&input, quests::updated_at.eq(utc_now())))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        if let Some(due) = input.due {
            if previous.series_id.is_some() {
                diesel::update(quests::table.find(id))
                    .set(quests::period_key.eq(Some(due)))
                    .execute(self.conn)
                    .map_err(|error| error.to_string())?;
            }
        }
        let completed_at = match status.as_deref() {
            Some("completed") => Some(Some(utc_now())),
            Some("active") | Some("abandoned") => Some(None),
            _ => None,
        };
        if let Some(completed_at) = completed_at {
            diesel::update(quests::table.find(id))
                .set(quests::completed_at.eq(completed_at))
                .execute(self.conn)
                .map_err(|error| error.to_string())?;
        }
        Ok((previous, self.get(id)?))
    }

    pub fn update_patch(
        &mut self,
        id: &str,
        patch: QuestUpdatePatch,
    ) -> Result<(Quest, Quest), String> {
        let QuestUpdatePatch {
            mut input,
            description,
            due,
            due_time,
            repeat_rule,
        } = patch;
        let clear_description = matches!(description, QuestFieldPatch::Clear);
        let clear_due = matches!(due, QuestFieldPatch::Clear);
        let clear_due_time = matches!(due_time, QuestFieldPatch::Clear);
        let clear_repeat_rule = matches!(repeat_rule, QuestFieldPatch::Clear);
        if let QuestFieldPatch::Set(value) = description {
            input.description = Some(value);
        }
        if let QuestFieldPatch::Set(value) = due {
            input.due = Some(value);
        }
        if let QuestFieldPatch::Set(value) = due_time {
            input.due_time = Some(value);
        }
        if let QuestFieldPatch::Set(value) = repeat_rule {
            input.repeat_rule = Some(value);
        }
        let (previous, mut quest) = self.update(id, input)?;
        if clear_description {
            self.clear_description(id)?;
        }
        if clear_due {
            self.clear_due(id)?;
        }
        if clear_due_time {
            self.clear_due_time(id)?;
        }
        if clear_repeat_rule {
            self.clear_repeat_rule(id)?;
        }
        if clear_description || clear_due || clear_due_time || clear_repeat_rule {
            quest = self.get(id)?;
        }
        Ok((previous, quest))
    }

    fn clear_description(&mut self, id: &str) -> Result<(), String> {
        diesel::update(quests::table.find(id))
            .set(quests::description.eq::<Option<String>>(None))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn clear_due(&mut self, id: &str) -> Result<(), String> {
        diesel::update(quests::table.find(id))
            .set((
                quests::due.eq::<Option<String>>(None),
                quests::period_key.eq::<Option<String>>(None),
            ))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn clear_due_time(&mut self, id: &str) -> Result<(), String> {
        diesel::update(quests::table.find(id))
            .set(quests::due_time.eq::<Option<String>>(None))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn clear_repeat_rule(&mut self, id: &str) -> Result<(), String> {
        diesel::update(quests::table.find(id))
            .set(quests::repeat_rule.eq::<Option<String>>(None))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn delete(&mut self, id: &str) -> Result<Quest, String> {
        let quest = crate::schema::quests::table
            .find(id)
            .select(Quest::as_select())
            .first(self.conn)
            .map_err(|error| error.to_string())?;
        diesel::delete(crate::schema::quests::table.find(id))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        Ok(quest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::QuestUpdatePatch;
    use crate::models::{QuestFieldPatch, UpdateQuestInput};
    use crate::services::db::open_db_at_path;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db_path() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        std::env::temp_dir().join(format!("fini-quest-repository-null-patch-{unique}.db"))
    }

    #[test]
    fn update_patch_writes_sql_null_without_overwriting_omitted_nullable_fields() {
        let db_path = temp_db_path();
        let mut conn = open_db_at_path(&db_path);
        let created = QuestRepository::new(&mut conn)
            .create(crate::models::CreateQuestInput {
                space_id: "1".to_string(),
                title: "nullable patch".to_string(),
                description: Some("clear me".to_string()),
                energy: "medium".to_string(),
                priority: 1,
                due: Some("2026-04-01".to_string()),
                due_time: Some("10:30".to_string()),
                repeat_rule: Some("weekly".to_string()),
                order_rank: None,
            })
            .expect("create quest");

        let (_, updated) = QuestRepository::new(&mut conn)
            .update_patch(
                &created.id,
                QuestUpdatePatch {
                    input: UpdateQuestInput {
                        space_id: None,
                        title: None,
                        description: None,
                        status: None,
                        energy: None,
                        priority: None,
                        pinned: None,
                        due: None,
                        due_time: None,
                        repeat_rule: None,
                        order_rank: None,
                    },
                    description: QuestFieldPatch::Clear,
                    due: QuestFieldPatch::Unchanged,
                    due_time: QuestFieldPatch::Unchanged,
                    repeat_rule: QuestFieldPatch::Unchanged,
                },
            )
            .expect("write nullable patch");

        assert_eq!(updated.description, None);
        assert_eq!(updated.due.as_deref(), Some("2026-04-01"));
        assert_eq!(updated.due_time.as_deref(), Some("10:30"));
        assert_eq!(updated.repeat_rule.as_deref(), Some("weekly"));
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn update_patch_clear_due_clears_series_period_key() {
        let db_path = temp_db_path();
        let mut conn = open_db_at_path(&db_path);
        let (created, series) = QuestRepository::new(&mut conn)
            .create_series_and_quest(crate::models::CreateQuestInput {
                space_id: "1".to_string(),
                title: "nullable series patch".to_string(),
                description: None,
                energy: "medium".to_string(),
                priority: 1,
                due: Some("2026-04-01".to_string()),
                due_time: None,
                repeat_rule: Some("weekly".to_string()),
                order_rank: None,
            })
            .expect("create series quest");

        let (_, updated) = QuestRepository::new(&mut conn)
            .update_patch(
                &created.id,
                QuestUpdatePatch {
                    input: UpdateQuestInput {
                        space_id: None,
                        title: None,
                        description: None,
                        status: None,
                        energy: None,
                        priority: None,
                        pinned: None,
                        due: None,
                        due_time: None,
                        repeat_rule: None,
                        order_rank: None,
                    },
                    description: QuestFieldPatch::Unchanged,
                    due: QuestFieldPatch::Clear,
                    due_time: QuestFieldPatch::Unchanged,
                    repeat_rule: QuestFieldPatch::Unchanged,
                },
            )
            .expect("clear nullable due patch");

        assert_eq!(updated.due, None);
        assert_eq!(updated.period_key, None);
        assert!(!QuestRepository::new(&mut conn)
            .series_has_occurrence(&series.id, "2026-04-01")
            .expect("check old period key"));
        let _ = std::fs::remove_file(db_path);
    }
}
