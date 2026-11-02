use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use std::collections::HashMap;

use crate::models::{
    clamp_order_rank, period_key_of, CreateQuestInput, Quest, QuestFieldPatch, QuestOccurrence, QuestUpdatePatch,
    RepeatRule, RepeatingQuest, Space, UpdateQuestInput, OCCURRENCE_ABANDONED, OCCURRENCE_COMPLETED,
};
use crate::repositories::OutboxRepository;
use crate::schema::{checklist_activity, focus_history, quest_occurrences, quests, spaces};
use crate::services::checklist;
use crate::services::db::utc_now;
use crate::services::quest::{
    append_focus_history, emit_occurrence_sync_event, emit_quest_sync_events, is_overdue, log_checklist_activity,
    parse_utc_timestamp,
    should_set_focus_now_for_restore, sort_quests_for_list, validate_metadata, validate_optional_metadata, EditScope, QuestChange,
    UpdateQuestInputBuilder,
};

/// The quests and the occurrence marks of repeating quests (ADR-0012): the
/// only code that reads and writes them.
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

    // "Focus N" badge counting is switched off for now; see the note in
    // services/quest.rs.
    // pub fn increment_focus_enter_count(&mut self, id: &str) -> Result<Quest, String> {
    //     diesel::update(quests::table.find(id))
    //         .set(quests::focus_enter_count.eq(quests::focus_enter_count + 1))
    //         .execute(self.conn)
    //         .map_err(|error| error.to_string())?;
    //     self.get(id)
    // }

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

    /// Writes a new quest row as given.
    pub fn insert(&mut self, input: CreateQuestInput) -> Result<Quest, String> {
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

    /// Writes the given fields of a quest row.
    pub fn write_fields(
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

    /// Writes the given fields of a quest row, and clears the ones the patch clears.
    pub fn write_patch(
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
        let (previous, mut quest) = self.write_fields(id, input)?;
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

    /// Deletes the quest; its occurrence marks go with it (foreign key cascade).
    pub fn delete(&mut self, id: &str) -> Result<Quest, String> {
        let quest = self.get(id)?;
        diesel::delete(quests::table.find(id))
            .execute(self.conn)
            .map_err(|error| error.to_string())?;
        Ok(quest)
    }
}


// ── Quests as a person sees and changes them (ADR-0012) ─────────────────────

/// The Focus fallback order (src/views/FocusView.md): overdue first, then
/// `order_rank`, then Priority (high first), then the oldest `created_at`.
fn fallback_order(mut quests: Vec<Quest>) -> Vec<Quest> {
    let now = Utc::now();
    // A `created_at` that does not read as a time goes last.
    let created = |quest: &Quest| parse_utc_timestamp(&quest.created_at).unwrap_or(DateTime::<Utc>::MAX_UTC);
    quests.sort_by(|a, b| {
        is_overdue(b, &now)
            .cmp(&is_overdue(a, &now))
            .then_with(|| a.order_rank.partial_cmp(&b.order_rank).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| b.priority.cmp(&a.priority))
            .then_with(|| created(a).cmp(&created(b)))
            .then_with(|| a.id.cmp(&b.id))
    });
    quests
}

/// A change of the status only.
fn status_patch(status: &str) -> QuestUpdatePatch {
    QuestUpdatePatch::unchanged(UpdateQuestInputBuilder::default().status(status).build())
}

impl<'a> QuestRepository<'a> {
    // ---- Reading: the resolved view (ADR-0012 D2, D6) ----

    /// Every quest as the UI lists it: one-time quests as stored; for a
    /// repeating quest its oldest open occurrence, and each closed day (the
    /// days in History).
    pub fn list_for_ui(&mut self) -> Result<Vec<Quest>, String> {
        let quests = self.load_all()?;
        let marks = self.all_occurrences()?;
        let mut listed = Vec::with_capacity(quests.len() + marks.len());
        for quest in &quests {
            match RepeatingQuest::new(quest, &marks) {
                Some(repeating) => {
                    if quest.status == "active" {
                        if let Some(period_key) = repeating.oldest_open() {
                            listed.push(repeating.on_day(&period_key));
                        }
                    }
                    for period_key in repeating.closed() {
                        listed.push(repeating.on_day(&period_key));
                    }
                }
                None => listed.push(quest.clone()),
            }
        }
        Ok(sort_quests_for_list(listed))
    }

    /// The quests that can have Focus: active one-time quests, and the oldest
    /// open occurrence of each active repeating quest.
    pub fn active_quests(&mut self) -> Result<Vec<Quest>, String> {
        Ok(self
            .list_for_ui()?
            .into_iter()
            .filter(|quest| quest.status == "active")
            .collect())
    }

    /// The Focus quest at `now` (src/views/FocusView.md): among the active
    /// quests that have come (`due_at` at or before `now`), the one with the
    /// newest Set Focus or `due_at`; with none, the first in the fallback
    /// order. It only reads.
    pub fn focus(&mut self, now: DateTime<Utc>) -> Result<Option<Quest>, String> {
        let candidates: Vec<Quest> = self
            .active_quests()?
            .into_iter()
            .filter(|quest| quest.due_at().is_none_or(|due_at| due_at <= now))
            .collect();
        let set_focus_at = self.last_set_focus_at()?;
        let newest = candidates
            .iter()
            .filter_map(|quest| {
                let moment = [set_focus_at.get(&quest.id).copied(), quest.due_at()].into_iter().flatten().max()?;
                Some((moment, quest))
            })
            .max_by_key(|(moment, _)| *moment)
            .map(|(_, quest)| quest.clone());
        Ok(newest.or_else(|| fallback_order(candidates).into_iter().next()))
    }

    /// The newest Set Focus of each quest (`focus_history`).
    fn last_set_focus_at(&mut self) -> Result<HashMap<String, DateTime<Utc>>, String> {
        let history: Vec<crate::models::FocusHistoryEntry> = focus_history::table
            .select(crate::models::FocusHistoryEntry::as_select())
            .load(self.conn)
            .map_err(|error| error.to_string())?;
        let mut moments: Vec<(String, DateTime<Utc>)> = history
            .into_iter()
            .filter_map(|entry| Some((entry.quest_id, parse_utc_timestamp(&entry.created_at)?)))
            .collect();
        // Oldest first, so the newest of each quest is the one the map keeps.
        moments.sort_by_key(|(_, at)| *at);
        Ok(moments.into_iter().collect())
    }

    /// The quest as the UI shows it now (see `list_for_ui`).
    pub fn resolved(&mut self, id: &str) -> Result<Quest, String> {
        let quest = self.get(id)?;
        let marks = self.occurrences(id)?;
        Ok(match RepeatingQuest::new(&quest, &marks) {
            Some(repeating) if quest.status == "active" => repeating
                .oldest_open()
                .map(|period_key| repeating.on_day(&period_key))
                .unwrap_or_else(|| quest.clone()),
            _ => quest.clone(),
        })
    }

    /// For a repeating quest: the open occurrence a reminder is for (the
    /// first one dated today or later, ADR-0012 D9), resolved.
    pub fn reminder_occurrence(&mut self, id: &str) -> Result<Option<Quest>, String> {
        let quest = self.get(id)?;
        let marks = self.occurrences(id)?;
        let Some(repeating) = RepeatingQuest::new(&quest, &marks) else {
            return Ok(None);
        };
        if quest.status != "active" {
            return Ok(None);
        }
        Ok(repeating
            .next_open_from(Utc::now().date_naive())
            .map(|period_key| repeating.on_day(&period_key)))
    }

    pub fn is_repeating(&mut self, id: &str) -> Result<bool, String> {
        let quest = self.get(id)?;
        Ok(RepeatRule::parse(quest.repeat_rule.as_deref()).is_some())
    }

    /// Focus on a quest; for a repeating quest on the quest itself, with no
    /// day row (ADR-0012 D6).
    pub fn set_focus(&mut self, id: &str, trigger: &str) -> Result<Quest, String> {
        let quest = self
            .get_active(id)?
            .ok_or_else(|| "cannot set Focus on non-active quest".to_string())?;
        self.touch(id)?;
        append_focus_history(self.conn, &quest.id, &quest.space_id, trigger)?;
        self.resolved(id)
    }

    pub fn create(&mut self, input: CreateQuestInput) -> Result<Quest, String> {
        validate_metadata(input.energy, input.priority)?;
        let repeats = RepeatRule::parse(input.repeat_rule.as_deref()).is_some();
        let input = if repeats && input.due.is_none() {
            // A repeating quest's rule starts at its date (ADR-0012 D2).
            CreateQuestInput { due: Some(period_key_of(Utc::now().date_naive())), ..input }
        } else {
            input
        };
        let quest = self.insert(input)?;
        self.resolved(&quest.id)
    }

    // ---- Writing ----

    /// Completes the quest; for a repeating quest its shown day (ADR-0012 D7).
    pub fn complete(&mut self, id: &str, origin_device_id: Option<&str>) -> Result<QuestChange, String> {
        self.update(id, status_patch(OCCURRENCE_COMPLETED), EditScope::All, None, origin_device_id)
    }

    /// Abandons the quest; for a repeating quest its shown day.
    pub fn abandon(&mut self, id: &str, origin_device_id: Option<&str>) -> Result<QuestChange, String> {
        self.update(id, status_patch(OCCURRENCE_ABANDONED), EditScope::All, None, origin_device_id)
    }

    /// One quest change. For a repeating quest (ADR-0012): Complete or Abandon
    /// closes one occurrence (`period_key`, or the oldest open); "active" on a
    /// closed day opens it again; `EditScope::Day` writes the fields as that
    /// day's overrides; `EditScope::All` edits the quest; clearing the rule
    /// stops the repeat (D10).
    pub fn update(
        &mut self,
        id: &str,
        patch: QuestUpdatePatch,
        scope: EditScope,
        period_key: Option<&str>,
        origin_device_id: Option<&str>,
    ) -> Result<QuestChange, String> {
        validate_optional_metadata(patch.input.energy, patch.input.priority)?;
        let quest = self.get(id)?;
        if !matches!(patch.description, QuestFieldPatch::Unchanged) || patch.input.description.is_some() {
            Self::reject_prose_patch_on_checklist(&quest)?;
        }
        let marks = self.occurrences(id)?;
        let Some(repeating) = RepeatingQuest::new(&quest, &marks) else {
            return self.update_one_time(quest, patch, origin_device_id);
        };

        let status = patch.input.status.clone();
        match status.as_deref() {
            Some(OCCURRENCE_COMPLETED) | Some(OCCURRENCE_ABANDONED) => {
                let period_key = match period_key {
                    Some(key) => key.to_string(),
                    None => repeating.oldest_open().ok_or_else(|| "the quest has no open occurrence".to_string())?,
                };
                let closed = self.close_occurrence(&repeating, &period_key, status.as_deref().unwrap(), origin_device_id)?;
                return self.finish_repeating(id, vec![closed], Vec::new(), false, Some(&period_key));
            }
            Some("active") => {
                if let Some(period_key) = period_key {
                    return self.reopen_occurrence(&repeating, period_key);
                }
            }
            _ => {}
        }

        if matches!(patch.repeat_rule, QuestFieldPatch::Clear) {
            // The rest of the same edit (a title, a date) applies too.
            let mut rest = patch;
            rest.repeat_rule = QuestFieldPatch::Unchanged;
            rest.input.status = None;
            self.write_patch(id, rest)?;
            let quest = self.get(id)?;
            let repeating = RepeatingQuest::new(&quest, &marks)
                .ok_or_else(|| "the quest does not repeat".to_string())?;
            return self.stop_repeat(&repeating, origin_device_id);
        }

        match scope {
            EditScope::Day => {
                let period_key = match period_key {
                    Some(key) => key.to_string(),
                    None => repeating.oldest_open().ok_or_else(|| "the quest has no open occurrence".to_string())?,
                };
                let mut mark = repeating.mark_or_new(&period_key);
                if let Some(title) = patch.input.title.clone() {
                    mark.title = (title != quest.title).then_some(title);
                }
                if let Some(energy) = patch.input.energy {
                    mark.energy = (energy != quest.energy).then_some(energy);
                }
                if let Some(priority) = patch.input.priority {
                    mark.priority = (priority != quest.priority).then_some(priority);
                }
                match &patch.due_time {
                    QuestFieldPatch::Set(time) => mark.due_time = (Some(time) != quest.due_time.as_ref()).then(|| time.clone()),
                    QuestFieldPatch::Clear => mark.due_time = None,
                    QuestFieldPatch::Unchanged => {}
                }
                if let QuestFieldPatch::Set(description) = &patch.description {
                    mark.description = Some(description.clone());
                }
                mark.updated_at = utc_now();
                let saved = self.save_occurrence(&mark)?;
                self.finish_repeating(id, vec![saved], Vec::new(), false, Some(&period_key))
            }
            EditScope::All => {
                let mut patch = patch;
                // The quest of a repeating series stays active; its days close.
                patch.input.status = None;
                let (_, _) = self.write_patch(id, patch)?;
                self.finish_repeating(id, Vec::new(), Vec::new(), true, None)
            }
        }
    }

    fn update_one_time(
        &mut self,
        previous: Quest,
        patch: QuestUpdatePatch,
        origin_device_id: Option<&str>,
    ) -> Result<QuestChange, String> {
        let (_, quest) = self.write_patch(&previous.id, patch)?;
        let restore_should_focus = previous.status != "active"
            && quest.status == "active"
            && should_set_focus_now_for_restore(quest.due.as_deref(), Utc::now());

        // Issue #128: record the completion moment of a checklist.
        if previous.status == "active"
            && matches!(quest.status.as_str(), "completed" | "abandoned")
            && quest.is_checklist
        {
            let (done, total) = checklist::counts(quest.description.as_deref());
            let _ = log_checklist_activity(
                self.conn,
                &quest.id,
                "completed_snapshot",
                &format!("Checklist at completion: {done}/{total} checked"),
                origin_device_id,
            );
        }

        Ok(QuestChange {
            quest,
            quest_row_changed: true,
            occurrences: Vec::new(),
            removed_occurrences: Vec::new(),
            restore_should_focus,
        })
    }

    /// Writes the closed mark of one day: its status, and for a checklist a
    /// copy of the day's items as they were (History, D5).
    fn close_occurrence(
        &mut self,
        repeating: &RepeatingQuest<'_>,
        period_key: &str,
        status: &str,
        origin_device_id: Option<&str>,
    ) -> Result<QuestOccurrence, String> {
        let quest = repeating.quest();
        let day = repeating.on_day(period_key);
        let now = utc_now();
        let mut mark = repeating.mark_or_new(period_key);
        mark.status = Some(status.to_string());
        mark.closed_at = Some(now.clone());
        mark.updated_at = now;
        if quest.is_checklist && mark.description.is_none() {
            mark.description = day.description.clone();
            mark.checklist_base = quest.description.clone();
        }
        if quest.is_checklist {
            let (done, total) = checklist::counts(day.description.as_deref());
            let _ = log_checklist_activity(
                self.conn,
                &quest.id,
                "completed_snapshot",
                &format!("Checklist at completion ({period_key}): {done}/{total} checked"),
                origin_device_id,
            );
        }
        self.save_occurrence(&mark)
    }

    /// Opens a closed day again. A mark with nothing else to keep goes.
    fn reopen_occurrence(&mut self, repeating: &RepeatingQuest<'_>, period_key: &str) -> Result<QuestChange, String> {
        let id = repeating.quest().id.clone();
        let mut mark = repeating
            .mark(period_key)
            .cloned()
            .ok_or_else(|| format!("{period_key} of this quest is not closed"))?;
        mark.status = None;
        mark.closed_at = None;
        mark.updated_at = utc_now();
        let keeps_something = mark.title.is_some()
            || mark.energy.is_some()
            || mark.priority.is_some()
            || mark.due_time.is_some()
            || (mark.description.is_some() && mark.description != repeating.quest().description.as_deref().map(checklist::reset_unchecked));
        if keeps_something {
            let saved = self.save_occurrence(&mark)?;
            self.finish_repeating(&id, vec![saved], Vec::new(), false, None)
        } else {
            self.delete_occurrence(&id, period_key)?;
            self.finish_repeating(&id, Vec::new(), vec![period_key.to_string()], false, None)
        }
    }

    /// ADR-0012 D10: with no marks the quest becomes a one-time quest on its
    /// date; with marks the rule gets an end at the last closed
    /// day, and the quest ends.
    fn stop_repeat(&mut self, repeating: &RepeatingQuest<'_>, origin_device_id: Option<&str>) -> Result<QuestChange, String> {
        let quest = repeating.quest().clone();
        let closed = repeating.closed();
        // With no closed day the quest's date is its oldest open day already.
        let input = match closed.last() {
            None => UpdateQuestInputBuilder::default().build(),
            Some(last_closed) => {
                let mut rule = repeating.rule().clone();
                rule.end = Some("on_date".to_string());
                rule.end_date = Some(last_closed.clone());
                rule.end_after = None;
                UpdateQuestInputBuilder::default()
                    .status("completed")
                    .repeat_rule(serde_json::to_string(&rule).map_err(|e| e.to_string())?)
                    .build()
            }
        };
        let clear_rule = closed.is_empty();
        let patch = QuestUpdatePatch {
            input,
            description: QuestFieldPatch::Unchanged,
            due: QuestFieldPatch::Unchanged,
            due_time: QuestFieldPatch::Unchanged,
            repeat_rule: if clear_rule { QuestFieldPatch::Clear } else { QuestFieldPatch::Unchanged },
        };
        let _ = origin_device_id;
        self.write_patch(&quest.id, patch)?;
        self.finish_repeating(&quest.id, Vec::new(), Vec::new(), true, None)
    }

    /// "Complete through today" (ADR-0012 D7): closes every open occurrence
    /// dated today or before, today's included.
    pub fn complete_through_today(&mut self, id: &str, origin_device_id: Option<&str>) -> Result<QuestChange, String> {
        let quest = self.get(id)?;
        let marks = self.occurrences(id)?;
        let repeating = RepeatingQuest::new(&quest, &marks).ok_or_else(|| "the quest does not repeat".to_string())?;
        let days = repeating.open_through(Utc::now().date_naive());
        if days.is_empty() {
            return Err("no open occurrence is dated today or before".to_string());
        }
        let mut closed = Vec::with_capacity(days.len());
        for period_key in &days {
            closed.push(self.close_occurrence(&repeating, period_key, OCCURRENCE_COMPLETED, origin_device_id)?);
        }
        self.finish_repeating(id, closed, Vec::new(), false, days.last().map(String::as_str))
    }

    fn finish_repeating(
        &mut self,
        id: &str,
        occurrences: Vec<QuestOccurrence>,
        removed_occurrences: Vec<String>,
        quest_row_changed: bool,
        changed_day: Option<&str>,
    ) -> Result<QuestChange, String> {
        let stored = self.get(id)?;
        let marks = self.occurrences(id)?;
        // The UI shows the oldest open day; with none left, the day that
        // changed.
        let quest = match RepeatingQuest::new(&stored, &marks) {
            Some(repeating) => {
                let open = (stored.status == "active").then(|| repeating.oldest_open()).flatten();
                match (open, changed_day) {
                    (Some(open), _) => repeating.on_day(&open),
                    (None, Some(day)) => repeating.on_day(day),
                    (None, None) => stored.clone(),
                }
            }
            None => stored.clone(),
        };
        Ok(QuestChange {
            quest,
            quest_row_changed,
            occurrences,
            removed_occurrences,
            restore_should_focus: false,
        })
    }

    /// A checklist quest's `description` IS the checklist text (issue #128) — an explicit
    /// description patch on a checklist quest would silently replace it with prose, losing
    /// every item's id and checked state. Callers must use the checklist item commands.
    fn reject_prose_patch_on_checklist(quest: &Quest) -> Result<(), String> {
        if quest.is_checklist {
            Err("quest is a checklist; use the checklist commands to edit its items instead of --description".to_string())
        } else {
            Ok(())
        }
    }

    // ---- Sync and backup ----

    pub fn sync_upsert_quest(&mut self, quest: &Quest) -> Result<(), String> {
        validate_metadata(quest.energy, quest.priority)?;
        let existing = self.external_quest_checklist_state(&quest.id)?;
        let mut persisted = quest.clone();
        persisted.checklist_base = match existing {
            Some((false, _)) if quest.is_checklist => quest.description.clone(),
            Some((_, base)) => base,
            None if quest.is_checklist => quest.description.clone(),
            None => None,
        };
        self.upsert_external_quest(&persisted)
    }

    /// A mark from a peer. A day's checklist copy merges with the local copy
    /// (the 3-way merge, D5); the other fields: the newer edit wins (the
    /// caller already compared the events).
    pub fn sync_upsert_occurrence(&mut self, incoming: &QuestOccurrence) -> Result<(), String> {
        let local = self.find_occurrence(&incoming.quest_id, &incoming.period_key)?;
        let mut merged = incoming.clone();
        if let Some(local) = local {
            merged.checklist_base = local.checklist_base.clone();
            if let (Some(local_description), Some(incoming_description)) = (&local.description, &incoming.description) {
                let (description, _) = checklist::merge_3way(local.checklist_base.as_deref(), local_description, incoming_description);
                merged.description = Some(description);
            }
        } else if merged.description.is_some() {
            merged.checklist_base = merged.description.clone();
        }
        self.save_occurrence(&merged).map(|_| ())
    }

    /// Emits an upsert for each repeating quest and occurrence mark that has
    /// no sync event yet (the rows a migration wrote, ADR-0012 D11).
    #[cfg(any(feature = "ui-plane", test))]
    pub fn emit_missing_sync_events(&mut self, device_id: &str) -> Result<(), String> {
        let quests = self.load_all()?;
        let marks = self.all_occurrences()?;
        let space_by_quest: HashMap<&str, &str> =
            quests.iter().map(|quest| (quest.id.as_str(), quest.space_id.as_str())).collect();
        let conn = &mut *self.conn;
        for quest in quests.iter().filter(|quest| RepeatRule::parse(quest.repeat_rule.as_deref()).is_some()) {
            if OutboxRepository::new(conn).load_latest_event_for_entity("quest", &quest.id)?.is_none() {
                emit_quest_sync_events(conn, device_id, &quest.space_id, &quest.id)?;
            }
        }
        for mark in &marks {
            let Some(space_id) = space_by_quest.get(mark.quest_id.as_str()) else { continue };
            if OutboxRepository::new(conn).load_latest_event_for_entity("quest_occurrence", &mark.sync_id())?.is_none() {
                emit_occurrence_sync_event(conn, device_id, space_id, mark)?;
            }
        }
        Ok(())
    }

    pub fn sync_merge_quest_checklist(
        &mut self,
        id: &str,
        incoming_description: Option<&str>,
    ) -> Result<(Option<String>, bool), String> {
        let local = self.find_external_quest(id)?;
        let local = match local {
            None => return Ok((incoming_description.map(str::to_string), false)),
            Some(local) => local,
        };
        let (merged_raw, _had_text_conflict) = checklist::merge_3way(
            local.checklist_base.as_deref(),
            local.description.as_deref().unwrap_or(""),
            incoming_description.unwrap_or(""),
        );
        let merged = if merged_raw.is_empty() {
            None
        } else {
            Some(merged_raw)
        };
        let checklist_base_before = local.checklist_base.clone();
        let incoming_matches_merged = incoming_description == merged.as_deref();
        let next_checklist_base = if incoming_matches_merged {
            merged.clone()
        } else {
            checklist_base_before.clone()
        };
        let base_advanced = checklist_base_before.as_deref() != next_checklist_base.as_deref();

        if merged != local.description {
            self.update_external_quest_checklist_merge(
                id,
                &merged,
                &next_checklist_base,
            )?;
        } else if base_advanced {
            self.update_external_quest_checklist_base(id, &next_checklist_base)?;
        }

        Ok((
            merged.clone(),
            merged.as_deref() != incoming_description || base_advanced,
        ))
    }


    // ── Checklist (issue #128; repeating quests: ADR-0012 D5) ──────────────────

    fn checklist_audit_kind(quest: &Quest, base_kind: &str) -> &'static str {
        // Every edit made after a quest has left "active" status is recorded distinctly so the
        // completed-history record stays auditable (#128: "Every post-completion edit is
        // recorded as historical activity").
        if quest.status != "active" {
            "post_completion_edit"
        } else {
            match base_kind {
                "added" => "added",
                "removed" => "removed",
                _ => "edited",
            }
        }
    }

    fn ensure_checklist_quest(quest: &Quest) -> Result<(), String> {
        if quest.is_checklist {
            Ok(())
        } else {
            Err("quest is not a checklist".to_string())
        }
    }

    /// Applies `edit` to a quest's checklist text. One-time quest: its
    /// description. Repeating quest, `EditScope::Day`: the day copy of
    /// `period_key` (or the oldest open day). Repeating quest,
    /// `EditScope::All`: the quest's template, merged into every open day copy.
    fn edit_checklist(
        &mut self,
        quest_id: &str,
        scope: EditScope,
        period_key: Option<&str>,
        edit: impl Fn(&str) -> Result<String, String>,
    ) -> Result<QuestChange, String> {
        let quest = self.get(quest_id)?;
        Self::ensure_checklist_quest(&quest)?;
        let marks = self.occurrences(quest_id)?;
        let Some(repeating) = RepeatingQuest::new(&quest, &marks) else {
            let updated = edit(quest.description.as_deref().unwrap_or(""))?;
            let quest = self.set_checklist_description(quest_id, &updated)?;
            return Ok(QuestChange {
                quest,
                quest_row_changed: true,
                occurrences: Vec::new(),
                removed_occurrences: Vec::new(),
                restore_should_focus: false,
            });
        };

        match scope {
            EditScope::Day => {
                let period_key = match period_key {
                    Some(key) => key.to_string(),
                    None => repeating.oldest_open().ok_or_else(|| "the quest has no open occurrence".to_string())?,
                };
                let day = repeating.on_day(&period_key);
                let updated = edit(day.description.as_deref().unwrap_or(""))?;
                let mut mark = repeating.mark_or_new(&period_key);
                if mark.description.is_none() {
                    mark.checklist_base = quest.description.clone();
                }
                mark.description = Some(updated);
                mark.updated_at = utc_now();
                let saved = self.save_occurrence(&mark)?;
                self.finish_repeating(quest_id, vec![saved], Vec::new(), false, Some(&period_key))
            }
            EditScope::All => {
                let template = quest.description.clone().unwrap_or_default();
                let updated = edit(&checklist::reset_unchecked(&template))?;
                self.set_checklist_description(quest_id, &updated)?;
                // The open day copies take the template edit (D5).
                let mut merged_days = Vec::new();
                for mark in marks.iter().filter(|mark| !mark.is_closed()) {
                    let Some(local) = mark.description.as_deref() else { continue };
                    let (description, _) = checklist::merge_3way(mark.checklist_base.as_deref(), local, &updated);
                    let mut mark = mark.clone();
                    mark.description = Some(description);
                    mark.checklist_base = Some(updated.clone());
                    mark.updated_at = utc_now();
                    merged_days.push(self.save_occurrence(&mark)?);
                }
                self.finish_repeating(quest_id, merged_days, Vec::new(), true, None)
            }
        }
    }

    pub fn add_checklist_item(
        &mut self,
        quest_id: &str,
        text: &str,
        scope: EditScope,
        origin_device_id: Option<&str>,
    ) -> Result<QuestChange, String> {
        let change = self.edit_checklist(quest_id, scope, None, |checklist_text| {
            Ok(checklist::add_item(Some(checklist_text).filter(|text| !text.is_empty()), text))
        })?;
        log_checklist_activity(
            self.conn,
            quest_id,
            Self::checklist_audit_kind(&change.quest, "added"),
            &format!("Added \"{text}\""),
            origin_device_id,
        )?;
        Ok(change)
    }

    /// A tick is always the day's (D8): it never asks for a scope.
    pub fn toggle_checklist_item(
        &mut self,
        quest_id: &str,
        item_id: &str,
        checked: bool,
        period_key: Option<&str>,
    ) -> Result<QuestChange, String> {
        let quest = self.resolved(quest_id)?;
        if quest.status != "active" && period_key.is_none() {
            return Err("checklist toggles are only available for active quests".to_string());
        }
        // Individual toggles are not written to the audit log by design (#128): the
        // checklist's own checked state is the record.
        self.edit_checklist(quest_id, EditScope::Day, period_key, |checklist_text| {
            if !checklist::parse(checklist_text).iter().any(|it| it.id == item_id) {
                return Err("checklist item not found".to_string());
            }
            Ok(checklist::set_checked(checklist_text, item_id, checked))
        })
    }

    pub fn edit_checklist_item_text(
        &mut self,
        quest_id: &str,
        item_id: &str,
        text: &str,
        scope: EditScope,
        origin_device_id: Option<&str>,
    ) -> Result<QuestChange, String> {
        // A stale/nonexistent item_id must not report a fake rename.
        let change = self.edit_checklist(quest_id, scope, None, |checklist_text| {
            if !checklist::parse(checklist_text).iter().any(|it| it.id == item_id) {
                return Err("checklist item not found".to_string());
            }
            Ok(checklist::set_text(checklist_text, item_id, text))
        })?;
        log_checklist_activity(
            self.conn,
            quest_id,
            Self::checklist_audit_kind(&change.quest, "edited"),
            &format!("Renamed item to \"{text}\""),
            origin_device_id,
        )?;
        Ok(change)
    }

    pub fn remove_checklist_item(
        &mut self,
        quest_id: &str,
        item_id: &str,
        scope: EditScope,
        origin_device_id: Option<&str>,
    ) -> Result<QuestChange, String> {
        let removed_text = std::cell::RefCell::new(None);
        let change = self.edit_checklist(quest_id, scope, None, |checklist_text| {
            let text = checklist::parse(checklist_text)
                .into_iter()
                .find(|it| it.id == item_id)
                .map(|it| it.text)
                .ok_or_else(|| "checklist item not found".to_string())?;
            *removed_text.borrow_mut() = Some(text);
            Ok(checklist::remove_item(checklist_text, item_id))
        })?;
        let removed_text = removed_text.into_inner().unwrap_or_default();
        log_checklist_activity(
            self.conn,
            quest_id,
            Self::checklist_audit_kind(&change.quest, "removed"),
            &format!("Removed \"{removed_text}\""),
            origin_device_id,
        )?;
        Ok(change)
    }

    /// Replaces the checklist text: the day copy (`EditScope::Day`) or the
    /// quest's template (`EditScope::All`).
    pub fn set_checklist(
        &mut self,
        quest_id: &str,
        checklist_text: &str,
        scope: EditScope,
        origin_device_id: Option<&str>,
    ) -> Result<QuestChange, String> {
        let before = self.resolved(quest_id)?;
        let change = self.edit_checklist(quest_id, scope, None, |_| Ok(checklist_text.to_string()))?;
        Self::log_checklist_replacement_activity(
            self.conn,
            &before,
            before.description.as_deref(),
            Some(checklist_text),
            origin_device_id,
        )?;
        Ok(change)
    }

    fn log_checklist_replacement_activity(
        conn: &mut SqliteConnection,
        quest: &Quest,
        old_checklist: Option<&str>,
        new_checklist: Option<&str>,
        origin_device_id: Option<&str>,
    ) -> Result<(), String> {
        let old_items = checklist::parse_opt(old_checklist);
        let new_items = checklist::parse_opt(new_checklist);
        let old_by_id: HashMap<&str, &checklist::ChecklistItem> =
            old_items.iter().map(|item| (item.id.as_str(), item)).collect();
        let new_by_id: HashMap<&str, &checklist::ChecklistItem> =
            new_items.iter().map(|item| (item.id.as_str(), item)).collect();

        for item in &new_items {
            match old_by_id.get(item.id.as_str()) {
                Some(old_item) if old_item.text != item.text => log_checklist_activity(
                    conn,
                    &quest.id,
                    Self::checklist_audit_kind(quest, "edited"),
                    &format!("Renamed item to \"{}\"", item.text),
                    origin_device_id,
                )?,
                None => log_checklist_activity(
                    conn,
                    &quest.id,
                    Self::checklist_audit_kind(quest, "added"),
                    &format!("Added \"{}\"", item.text),
                    origin_device_id,
                )?,
                _ => {}
            }
        }
        for item in &old_items {
            if !new_by_id.contains_key(item.id.as_str()) {
                log_checklist_activity(
                    conn,
                    &quest.id,
                    Self::checklist_audit_kind(quest, "removed"),
                    &format!("Removed \"{}\"", item.text),
                    origin_device_id,
                )?;
            }
        }
        Ok(())
    }

    pub fn reorder_checklist(
        &mut self,
        quest_id: &str,
        ordered_ids: &[String],
        scope: EditScope,
    ) -> Result<QuestChange, String> {
        self.edit_checklist(quest_id, scope, None, |checklist_text| Ok(checklist::reorder(checklist_text, ordered_ids)))
    }

    pub fn get_checklist_activity(
        &mut self,
        quest_id: &str,
    ) -> Result<Vec<crate::models::ChecklistActivity>, String> {
        checklist_activity::table
            .filter(checklist_activity::quest_id.eq(quest_id))
            .order(checklist_activity::created_at.asc())
            .select(crate::models::ChecklistActivity::as_select())
            .load(self.conn)
            .map_err(|e| e.to_string())
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
            .insert(crate::models::CreateQuestInput {
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
            .write_patch(
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
