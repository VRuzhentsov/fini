use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::models::{
    clamp_order_rank, CreateQuestInput, Quest, QuestFieldPatch, QuestOccurrence, QuestUpdatePatch, Space,
    UpdateQuestInput,
};
use crate::schema::{quest_occurrences, quests, spaces};
use crate::services::db::utc_now;

/// Diesel persistence boundary for quest records and the occurrence marks of
/// repeating quests (ADR-0012). Lifecycle policies remain in QuestService.
pub struct QuestRepository<'a> {
    pub(crate) conn: &'a mut SqliteConnection,
}

impl<'a> QuestRepository<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn load_all(&mut self) -> Result<Vec<Quest>, String> {
        quests::table
            .select(Quest::as_select())
            .load(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn load_for_spaces(&mut self, space_ids: &[String]) -> Result<Vec<Quest>, String> {
        quests::table
            .filter(quests::space_id.eq_any(space_ids))
            .select(Quest::as_select())
            .load(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn insert_export_quest(&mut self, quest: &Quest) -> Result<(), String> {
        diesel::insert_into(quests::table)
            .values((
                quests::id.eq(&quest.id),
                quests::space_id.eq(&quest.space_id),
                quests::title.eq(&quest.title),
                quests::description.eq(&quest.description),
                quests::status.eq(&quest.status),
                quests::energy.eq(quest.energy),
                quests::priority.eq(quest.priority),
                quests::due.eq(&quest.due),
                quests::due_time.eq(&quest.due_time),
                quests::repeat_rule.eq(&quest.repeat_rule),
                quests::completed_at.eq(&quest.completed_at),
                quests::order_rank.eq(quest.order_rank),
                quests::focus_enter_count.eq(quest.focus_enter_count),
                quests::created_at.eq(&quest.created_at),
                quests::updated_at.eq(&quest.updated_at),
                quests::is_checklist.eq(quest.is_checklist),
                quests::checklist_base.eq(&quest.checklist_base),
            ))
            .execute(self.conn)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn get(&mut self, id: &str) -> Result<Quest, String> {
        quests::table
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

    // ---- Occurrence marks (ADR-0012 D3) ----

    /// The marks of one quest, oldest date first.
    pub fn occurrences(&mut self, quest_id: &str) -> Result<Vec<QuestOccurrence>, String> {
        quest_occurrences::table
            .filter(quest_occurrences::quest_id.eq(quest_id))
            .order(quest_occurrences::period_key.asc())
            .select(QuestOccurrence::as_select())
            .load(self.conn)
            .map_err(|error| error.to_string())
    }

    /// Every mark, oldest date first.
    pub fn all_occurrences(&mut self) -> Result<Vec<QuestOccurrence>, String> {
        quest_occurrences::table
            .order((quest_occurrences::quest_id.asc(), quest_occurrences::period_key.asc()))
            .select(QuestOccurrence::as_select())
            .load(self.conn)
            .map_err(|error| error.to_string())
    }

    pub fn find_occurrence(&mut self, quest_id: &str, period_key: &str) -> Result<Option<QuestOccurrence>, String> {
        quest_occurrences::table
            .find((quest_id, period_key))
            .select(QuestOccurrence::as_select())
            .first(self.conn)
            .optional()
            .map_err(|error| error.to_string())
    }

    /// Writes a mark whole: inserts it, or replaces the stored one.
    pub fn save_occurrence(&mut self, occurrence: &QuestOccurrence) -> Result<QuestOccurrence, String> {
        diesel::insert_into(quest_occurrences::table)
            .values(occurrence)
            .on_conflict((quest_occurrences::quest_id, quest_occurrences::period_key))
            .do_update()
            .set(occurrence)
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        self.find_occurrence(&occurrence.quest_id, &occurrence.period_key)?
            .ok_or_else(|| "the occurrence was not saved".to_string())
    }

    pub fn delete_occurrence(&mut self, quest_id: &str, period_key: &str) -> Result<(), String> {
        diesel::delete(quest_occurrences::table.find((quest_id, period_key)))
            .execute(self.conn)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn count_completed_occurrences(&mut self, quest_id: &str) -> Result<i64, String> {
        quest_occurrences::table
            .filter(quest_occurrences::quest_id.eq(quest_id))
            .filter(quest_occurrences::status.eq(crate::models::OCCURRENCE_COMPLETED))
            .count()
            .get_result(self.conn)
            .map_err(|error| error.to_string())
    }

    /// Sets `description` (as checklist text) for a local edit — not a sync-merge apply.
    /// Marks the quest `is_checklist` if it wasn't already. `checklist_base` is deliberately
    /// left untouched here — it only advances when a sync merge completes, so the next merge
    /// can still tell that this device has diverged from the last mutually-agreed state.
    pub fn set_checklist_description(
        &mut self,
        quest_id: &str,
        checklist: &str,
    ) -> Result<Quest, String> {
        diesel::update(quests::table.find(quest_id))
            .set((
                quests::description.eq(checklist),
                quests::is_checklist.eq(true),
                quests::updated_at.eq(utc_now()),
            ))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        self.get(quest_id)
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

    pub fn find_external_quest(&mut self, id: &str) -> Result<Option<Quest>, String> {
        quests::table
            .find(id)
            .select(Quest::as_select())
            .first(self.conn)
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn external_quest_checklist_state(
        &mut self,
        id: &str,
    ) -> Result<Option<(bool, Option<String>)>, String> {
        quests::table
            .find(id)
            .select((quests::is_checklist, quests::checklist_base))
            .first(self.conn)
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn external_quest_is_checklist(&mut self, id: &str) -> Result<bool, String> {
        quests::table
            .find(id)
            .select(quests::is_checklist)
            .first(self.conn)
            .optional()
            .map(|value| value.unwrap_or(false))
            .map_err(|error| error.to_string())
    }

    pub fn update_external_quest_checklist_merge(
        &mut self,
        id: &str,
        description: &Option<String>,
        checklist_base: &Option<String>,
    ) -> Result<(), String> {
        diesel::update(quests::table.find(id))
            .set((
                quests::description.eq(description),
                quests::checklist_base.eq(checklist_base),
            ))
            .execute(self.conn)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn update_external_quest_checklist_base(
        &mut self,
        id: &str,
        checklist_base: &Option<String>,
    ) -> Result<(), String> {
        diesel::update(quests::table.find(id))
            .set(quests::checklist_base.eq(checklist_base))
            .execute(self.conn)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn move_quests_between_spaces(
        &mut self,
        local_space_id: &str,
        remote_space_id: &str,
    ) -> Result<(), String> {
        diesel::update(quests::table.filter(quests::space_id.eq(local_space_id)))
            .set(quests::space_id.eq(remote_space_id))
            .execute(self.conn)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn upsert_external_quest(&mut self, quest: &Quest) -> Result<(), String> {
        diesel::insert_into(quests::table)
            .values((
                quests::id.eq(&quest.id),
                quests::space_id.eq(&quest.space_id),
                quests::title.eq(&quest.title),
                quests::description.eq(&quest.description),
                quests::status.eq(&quest.status),
                quests::energy.eq(quest.energy),
                quests::priority.eq(quest.priority),
                quests::due.eq(&quest.due),
                quests::due_time.eq(&quest.due_time),
                quests::repeat_rule.eq(&quest.repeat_rule),
                quests::completed_at.eq(&quest.completed_at),
                quests::order_rank.eq(quest.order_rank),
                quests::focus_enter_count.eq(quest.focus_enter_count),
                quests::created_at.eq(&quest.created_at),
                quests::updated_at.eq(&quest.updated_at),
                quests::is_checklist.eq(quest.is_checklist),
                quests::checklist_base.eq(&quest.checklist_base),
            ))
            .on_conflict(quests::id)
            .do_update()
            .set((
                quests::space_id.eq(&quest.space_id),
                quests::title.eq(&quest.title),
                quests::description.eq(&quest.description),
                quests::status.eq(&quest.status),
                quests::energy.eq(quest.energy),
                quests::priority.eq(quest.priority),
                quests::due.eq(&quest.due),
                quests::due_time.eq(&quest.due_time),
                quests::repeat_rule.eq(&quest.repeat_rule),
                quests::completed_at.eq(&quest.completed_at),
                quests::order_rank.eq(quest.order_rank),
                quests::focus_enter_count.eq(quest.focus_enter_count),
                quests::created_at.eq(&quest.created_at),
                quests::updated_at.eq(&quest.updated_at),
                quests::is_checklist.eq(quest.is_checklist),
                quests::checklist_base.eq(&quest.checklist_base),
            ))
            .execute(self.conn)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn delete_external_quest(&mut self, id: &str) -> Result<(), String> {
        diesel::delete(quests::table.find(id))
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
        let quest: Quest = diesel::insert_into(quests::table)
            .values(&input)
            .returning(Quest::as_returning())
            .get_result(self.conn)
            .map_err(|error| error.to_string())?;
        // checklist_base is device-local convergence bookkeeping (never part of the Insertable
        // input) — seed it to match the freshly-created description so the first sync exchange
        // has a real base to diff against instead of treating any incoming edit as
        // unconditionally authoritative.
        if quest.is_checklist {
            diesel::update(quests::table.find(&quest.id))
                .set(quests::checklist_base.eq(&quest.description))
                .execute(self.conn)
                .map_err(|error| error.to_string())?;
            return self.get(&quest.id);
        }
        Ok(quest)
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
        let status = input.status.clone();
        diesel::update(quests::table.find(id))
            .set((&input, quests::updated_at.eq(utc_now())))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
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
            diesel::update(quests::table.find(id))
                .set(quests::description.eq::<Option<String>>(None))
                .execute(self.conn)
                .map_err(|error| error.to_string())?;
        }
        if clear_due {
            diesel::update(quests::table.find(id))
                .set(quests::due.eq::<Option<String>>(None))
                .execute(self.conn)
                .map_err(|error| error.to_string())?;
        }
        if clear_due_time {
            diesel::update(quests::table.find(id))
                .set(quests::due_time.eq::<Option<String>>(None))
                .execute(self.conn)
                .map_err(|error| error.to_string())?;
        }
        if clear_repeat_rule {
            diesel::update(quests::table.find(id))
                .set(quests::repeat_rule.eq::<Option<String>>(None))
                .execute(self.conn)
                .map_err(|error| error.to_string())?;
        }
        if clear_description || clear_due || clear_due_time || clear_repeat_rule {
            quest = self.get(id)?;
        }
        Ok((previous, quest))
    }

    pub fn delete(&mut self, id: &str) -> Result<Quest, String> {
        let quest = self.get(id)?;
        diesel::delete(quests::table.find(id))
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
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: Some("2026-04-01".to_string()),
                due_time: Some("10:30".to_string()),
                repeat_rule: Some("weekly".to_string()),
                order_rank: None,
                is_checklist: false,
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
                        due: None,
                        due_time: None,
                        repeat_rule: None,
                        order_rank: None,
                        is_checklist: None,
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
}
