use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use std::collections::HashMap;
#[cfg(any(feature = "ui-plane", test))]
use tauri::{Manager, State};

#[cfg(any(feature = "ui-plane", test))]
use crate::models::Space;
#[cfg(any(feature = "ui-plane", test))]
use crate::models::UpdateQuestInput;
#[cfg(any(feature = "ui-plane", test))]
use crate::models::UpdateQuestCommandInput;
use crate::models::{
    period_key_of, CreateFocusHistoryInput, Quest, QuestOccurrence, RepeatRule, RepeatingQuest, OCCURRENCE_ABANDONED,
    OCCURRENCE_COMPLETED,
};
use crate::models::{CreateQuestInput, QuestFieldPatch, QuestUpdatePatch};
use crate::repositories::QuestRepository;
use crate::schema::{checklist_activity, focus_history, settings};
#[cfg(test)]
use crate::schema::quests;
use crate::services::checklist;
use crate::services::db::utc_now;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::sync_timestamp;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::AppDbConnection;
#[cfg(any(feature = "ui-plane", test))]
use crate::repositories::OutboxRepository;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::reminder;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::space_sync_service::SpaceSyncService;


// ── Focus resolution ─────────────────────────────────────────────────────────

pub fn parse_utc_timestamp(value: &str) -> Option<DateTime<Utc>> {
    if let Ok(ts) = DateTime::parse_from_rfc3339(value) {
        return Some(ts.with_timezone(&Utc));
    }
    if let Ok(ts) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return Some(ts.and_utc());
    }
    if let Ok(ts) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S") {
        return Some(ts.and_utc());
    }
    None
}

fn parse_due_deadline_utc(quest: &Quest) -> Option<DateTime<Utc>> {
    let due = quest.due.as_deref()?;
    let date = NaiveDate::parse_from_str(due, "%Y-%m-%d").ok()?;
    let time = match quest.due_time.as_deref() {
        Some(value) => NaiveTime::parse_from_str(value, "%H:%M")
            .or_else(|_| NaiveTime::parse_from_str(value, "%H:%M:%S"))
            .ok()?,
        None => NaiveTime::from_hms_opt(23, 59, 59)?,
    };
    Some(NaiveDateTime::new(date, time).and_utc())
}

fn parse_due_reminder_fire_utc(quest: &Quest) -> Option<DateTime<Utc>> {
    let due = quest.due.as_deref()?;
    crate::services::due_time::compute_fire_utc(due, quest.due_time.as_deref())
}

pub fn is_overdue(quest: &Quest, now: &DateTime<Utc>) -> bool {
    parse_due_deadline_utc(quest)
        .map(|deadline| deadline < *now)
        .unwrap_or(false)
}

fn resolve_active_fallback(active: Vec<&Quest>) -> Option<Quest> {
    let now = Utc::now();
    let mut fallback = active;
    fallback.sort_by(|a, b| {
        let a_overdue = is_overdue(a, &now);
        let b_overdue = is_overdue(b, &now);

        if a_overdue != b_overdue {
            return b_overdue.cmp(&a_overdue);
        }

        if (a.order_rank - b.order_rank).abs() > f64::EPSILON {
            return a
                .order_rank
                .partial_cmp(&b.order_rank)
                .unwrap_or(std::cmp::Ordering::Equal);
        }

        if a.priority != b.priority {
            return b.priority.cmp(&a.priority);
        }

        match (
            parse_utc_timestamp(&a.created_at),
            parse_utc_timestamp(&b.created_at),
        ) {
            (Some(a_created), Some(b_created)) if a_created != b_created => {
                a_created.cmp(&b_created)
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => a.created_at.cmp(&b.created_at),
        }
        .then_with(|| a.id.cmp(&b.id))
    });

    fallback.first().map(|quest| (*quest).clone())
}

fn resolve_active_quest_at(
    conn: &mut SqliteConnection,
    now: DateTime<Utc>,
) -> Result<Option<Quest>, String> {
    let active_quests: Vec<Quest> = QuestService::new(conn)
        .active_quests()?
        .into_iter()
        // A quest is on Focus from its due + due_time (FocusView.md).
        .filter(|quest| parse_due_reminder_fire_utc(quest).is_none_or(|due_at| due_at <= now))
        .collect();
    let active_by_id: std::collections::HashMap<&str, &Quest> = active_quests
        .iter()
        .map(|q| (q.id.as_str(), q))
        .collect();

    if active_by_id.is_empty() {
        return Ok(None);
    }

    let mut winner: Option<(DateTime<Utc>, &Quest)> = None;

    // Walk persisted focus events; active reminder due times below compete at the same priority.
    let history: Vec<crate::models::FocusHistoryEntry> = focus_history::table
        .order(focus_history::created_at.desc())
        .select(crate::models::FocusHistoryEntry::as_select())
        .load(conn)
        .map_err(|error| error.to_string())?;

    for entry in &history {
        if let Some(&quest) = active_by_id.get(entry.quest_id.as_str()) {
            if let Some(created_at) = parse_utc_timestamp(&entry.created_at) {
                if winner
                    .as_ref()
                    .map(|(winner_at, _)| created_at > *winner_at)
                    .unwrap_or(true)
                {
                    winner = Some((created_at, quest));
                }
            }
        }
    }

    for quest in active_by_id.values().copied() {
        if let Some(fire_at) = parse_due_reminder_fire_utc(quest) {
            if fire_at <= now
                && winner
                    .as_ref()
                    .map(|(winner_at, _)| fire_at > *winner_at)
                    .unwrap_or(true)
            {
                winner = Some((fire_at, quest));
            }
        }
    }

    if let Some((_, quest)) = winner {
        return Ok(Some(quest.clone()));
    }

    // No focus_history or due-reminder candidate matches an active quest — use fallback ordering
    Ok(resolve_active_fallback(
        active_by_id.values().copied().collect(),
    ))
}

pub fn resolve_active_quest(conn: &mut SqliteConnection) -> Result<Option<Quest>, String> {
    resolve_active_quest_at(conn, Utc::now())
}

const ACTIVE_FOCUS_QUEST_SETTING_KEY: &str = "active_focus_quest_id";

fn save_active_focus_quest_id(
    conn: &mut SqliteConnection,
    quest_id: Option<&str>,
) -> Result<(), diesel::result::Error> {
    if let Some(quest_id) = quest_id {
        diesel::insert_into(settings::table)
            .values((
                settings::key.eq(ACTIVE_FOCUS_QUEST_SETTING_KEY),
                settings::value.eq(quest_id),
            ))
            .on_conflict(settings::key)
            .do_update()
            .set(settings::value.eq(quest_id))
            .execute(conn)?;
    } else {
        diesel::delete(settings::table.find(ACTIVE_FOCUS_QUEST_SETTING_KEY)).execute(conn)?;
    }
    Ok(())
}

/// Counts a Focus enter on the quest (ADR-0012 D9: a repeating quest counts
/// on the quest, not on a day) and returns `quest` with the new count.
pub(crate) fn record_focus_enter(
    conn: &mut SqliteConnection,
    quest: &Quest,
) -> Result<Quest, String> {
    let previous_focus_id = settings::table
        .find(ACTIVE_FOCUS_QUEST_SETTING_KEY)
        .select(settings::value)
        .first::<String>(conn)
        .optional()
        .map_err(|error| error.to_string())?;

    let mut quest = quest.clone();
    if previous_focus_id.as_deref() != Some(quest.id.as_str()) {
        let stored = QuestRepository::new(conn).increment_focus_enter_count(&quest.id)?;
        save_active_focus_quest_id(conn, Some(&quest.id)).map_err(|error| error.to_string())?;
        quest.focus_enter_count = stored.focus_enter_count;
    }
    Ok(quest)
}

fn resolve_and_record_active_quest(conn: &mut SqliteConnection) -> Result<Option<Quest>, String> {
    let Some(quest) = resolve_active_quest(conn)? else {
        save_active_focus_quest_id(conn, None).map_err(|error| error.to_string())?;
        return Ok(None);
    };

    record_focus_enter(conn, &quest).map(Some)
}

fn should_set_focus_now_for_restore(due: Option<&str>, now: DateTime<Utc>) -> bool {
    let Some(due_str) = due else {
        return true;
    };

    let Some(due_date) = NaiveDate::parse_from_str(due_str, "%Y-%m-%d").ok() else {
        return true;
    };

    due_date <= now.date_naive()
}

pub(crate) fn append_focus_history(
    conn: &mut SqliteConnection,
    quest_id: &str,
    space_id: &str,
    trigger: &str,
) -> Result<(), String> {
    let input = CreateFocusHistoryInput {
        quest_id: quest_id.to_string(),
        space_id: space_id.to_string(),
        trigger: trigger.to_string(),
    };

    diesel::insert_into(focus_history::table)
        .values(&input)
        .execute(conn)
        .map_err(|e| e.to_string())?;

    Ok(())
}

// ── Sync events ──────────────────────────────────────────────────────────────

/// Emits the stored quest row (never a resolved occurrence view: a repeating
/// quest syncs as one entity, ADR-0012 D11).
#[cfg(any(feature = "ui-plane", test))]
fn emit_quest_sync_events(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    previous_space_id: &str,
    quest_id: &str,
) -> Result<(), String> {
    let quest = QuestRepository::new(conn).get(quest_id)?;
    let payload = serde_json::to_string(&quest).map_err(|e| e.to_string())?;

    if previous_space_id == quest.space_id {
        OutboxRepository::new(conn).emit_sync_event(
            origin_device_id,
            "quest",
            &quest.id,
            &quest.space_id,
            "upsert",
            Some(payload),
        )?;
        return emit_checklist_activity_sync_events(conn, origin_device_id, &quest);
    }

    let delete_updated_at = sync_timestamp();
    let upsert_updated_at = (Utc::now() + chrono::Duration::seconds(1))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    OutboxRepository::new(conn).emit_sync_event_at(
        origin_device_id,
        "quest",
        &quest.id,
        previous_space_id,
        "delete",
        None,
        delete_updated_at,
    )?;

    OutboxRepository::new(conn).emit_sync_event_at(
        origin_device_id,
        "quest",
        &quest.id,
        &quest.space_id,
        "upsert",
        Some(payload),
        upsert_updated_at,
    )?;

    emit_checklist_activity_sync_events(conn, origin_device_id, &quest)?;

    // The marks go with their quest to its new space.
    for occurrence in QuestRepository::new(conn).occurrences(&quest.id)? {
        emit_occurrence_sync_event(conn, origin_device_id, &quest.space_id, &occurrence)?;
    }

    Ok(())
}

/// Emits one occurrence mark: the entity `quest_occurrence`, keyed by
/// (quest_id, period_key) (ADR-0012 D11).
#[cfg(any(feature = "ui-plane", test))]
fn emit_occurrence_sync_event(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    space_id: &str,
    occurrence: &QuestOccurrence,
) -> Result<(), String> {
    let payload = serde_json::to_string(occurrence).map_err(|e| e.to_string())?;
    OutboxRepository::new(conn).emit_sync_event(
        origin_device_id,
        "quest_occurrence",
        &occurrence.sync_id(),
        space_id,
        "upsert",
        Some(payload),
    )
}

/// Emits a removed mark (a day opened again with nothing left to keep).
#[cfg(any(feature = "ui-plane", test))]
fn emit_occurrence_delete_event(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    space_id: &str,
    quest_id: &str,
    period_key: &str,
) -> Result<(), String> {
    OutboxRepository::new(conn).emit_sync_event(
        origin_device_id,
        "quest_occurrence",
        &format!("{quest_id}:{period_key}"),
        space_id,
        "delete",
        None,
    )
}

#[cfg(any(feature = "ui-plane", test))]
fn emit_checklist_activity_sync_events(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    quest: &Quest,
) -> Result<(), String> {
    if !quest.is_checklist {
        return Ok(());
    }

    let emitted_activity_ids: Vec<String> = crate::schema::sync_outbox::table
        .filter(crate::schema::sync_outbox::entity_type.eq("checklist_activity"))
        .filter(crate::schema::sync_outbox::op_type.eq("upsert"))
        .filter(crate::schema::sync_outbox::space_id.eq(&quest.space_id))
        .select(crate::schema::sync_outbox::entity_id)
        .load(conn)
        .map_err(|e| e.to_string())?;

    let activities: Vec<crate::models::ChecklistActivity> = checklist_activity::table
        .filter(checklist_activity::quest_id.eq(&quest.id))
        .filter(
            checklist_activity::origin_device_id
                .is_null()
                .or(checklist_activity::origin_device_id.eq(origin_device_id)),
        )
        .filter(checklist_activity::id.ne_all(emitted_activity_ids))
        .select(crate::models::ChecklistActivity::as_select())
        .load(conn)
        .map_err(|e| e.to_string())?;

    for activity in activities {
        let payload = serde_json::to_string(&activity).map_err(|e| e.to_string())?;
        OutboxRepository::new(conn).emit_sync_event(
            origin_device_id,
            "checklist_activity",
            &activity.id,
            &quest.space_id,
            "upsert",
            Some(payload),
        )?;
    }

    Ok(())
}

/// Emits what a change wrote: the quest row, and each changed or removed mark.
#[cfg(any(feature = "ui-plane", test))]
fn emit_change_sync_events(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    previous_space_id: &str,
    change: &QuestChange,
) -> Result<(), String> {
    if change.quest_row_changed {
        emit_quest_sync_events(conn, origin_device_id, previous_space_id, &change.quest.id)?;
    }
    let space_id = QuestRepository::new(conn).get(&change.quest.id)?.space_id;
    for occurrence in &change.occurrences {
        emit_occurrence_sync_event(conn, origin_device_id, &space_id, occurrence)?;
    }
    for period_key in &change.removed_occurrences {
        emit_occurrence_delete_event(conn, origin_device_id, &space_id, &change.quest.id, period_key)?;
    }
    if !change.quest_row_changed && !change.occurrences.is_empty() {
        // Checklist activity of a day is kept on the quest.
        let quest = QuestRepository::new(conn).get(&change.quest.id)?;
        emit_checklist_activity_sync_events(conn, origin_device_id, &quest)?;
    }
    Ok(())
}

fn validate_metadata(energy: i64, priority: i64) -> Result<(), String> {
    if !matches!(energy, 1..=3) {
        return Err(format!(
            "invalid stored energy {energy}; expected 1, 2, or 3"
        ));
    }
    if !matches!(priority, 1..=3) {
        return Err(format!(
            "invalid stored priority {priority}; expected 1, 2, or 3"
        ));
    }
    Ok(())
}

fn validate_optional_metadata(energy: Option<i64>, priority: Option<i64>) -> Result<(), String> {
    validate_metadata(
        energy.unwrap_or(crate::models::ENERGY_MEDIUM),
        priority.unwrap_or(crate::models::PRIORITY_MEDIUM),
    )
}

/// What a quest change wrote, for the callers that sync it and schedule
/// reminders.
pub(crate) struct QuestChange {
    /// The quest as the UI shows it after the change: for a repeating quest,
    /// its oldest open occurrence (or the day that changed, if that one is
    /// closed now and nothing is open).
    pub quest: Quest,
    /// True when the stored quest row changed (sync emits it).
    pub quest_row_changed: bool,
    /// The marks the change wrote.
    pub occurrences: Vec<QuestOccurrence>,
    /// The days whose marks the change removed.
    pub removed_occurrences: Vec<String>,
    pub restore_should_focus: bool,
}

/// The edit scope of a repeating quest (ADR-0012 D8).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditScope {
    /// The one occurrence: an override on its mark.
    Day,
    /// The quest: every occurrence.
    All,
}

impl EditScope {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value {
            None | Some("all") => Ok(Self::All),
            Some("day") => Ok(Self::Day),
            Some(other) => Err(format!("invalid edit scope {other:?}; expected \"day\" or \"all\"")),
        }
    }
}

/// Application boundary for quest lifecycle. Adapters call this, never repositories.
pub struct QuestService<'a> {
    repository: QuestRepository<'a>,
}

impl<'a> QuestService<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self {
            repository: QuestRepository::new(conn),
        }
    }

    // ---- Reading: the resolved view (ADR-0012 D2, D6) ----

    /// Every quest as the UI lists it: one-time quests as stored; for a
    /// repeating quest its oldest open occurrence, and each closed day (the
    /// days in History).
    pub fn list_for_ui(&mut self) -> Result<Vec<Quest>, String> {
        let quests = self.repository.load_all()?;
        let marks = self.repository.all_occurrences()?;
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

    /// The quest as the UI shows it now (see `list_for_ui`).
    pub fn resolved(&mut self, id: &str) -> Result<Quest, String> {
        let quest = self.repository.get(id)?;
        let marks = self.repository.occurrences(id)?;
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
        let quest = self.repository.get(id)?;
        let marks = self.repository.occurrences(id)?;
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
        let quest = self.repository.get(id)?;
        Ok(RepeatRule::parse(quest.repeat_rule.as_deref()).is_some())
    }

    pub fn export_quests_for_spaces(&mut self, space_ids: &[String]) -> Result<Vec<Quest>, String> {
        self.repository.load_for_spaces(space_ids)
    }

    pub fn export_insert_quest(&mut self, quest: &Quest) -> Result<(), String> {
        validate_metadata(quest.energy, quest.priority)?;
        self.repository.insert_export_quest(quest)
    }

    pub fn import_load_quests(&mut self) -> Result<Vec<Quest>, String> {
        self.repository.load_all()
    }

    /// The stored quest row.
    pub fn get(&mut self, id: &str) -> Result<Quest, String> {
        self.repository.get(id)
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn get_with_space(&mut self, id: &str) -> Result<(Quest, Space), String> {
        self.repository.load_quest_and_space(id)
    }

    /// Focus on a quest; for a repeating quest on the quest itself, with no
    /// day row (ADR-0012 D6).
    pub fn set_focus(&mut self, id: &str, trigger: &str) -> Result<Quest, String> {
        let quest = self
            .repository
            .get_active(id)?
            .ok_or_else(|| "cannot set Focus on non-active quest".to_string())?;
        self.repository.touch(id)?;
        append_focus_history(self.repository.conn, &quest.id, &quest.space_id, trigger)?;
        let resolved = self.resolved(id)?;
        record_focus_enter(self.repository.conn, &resolved)
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
        let quest = self.repository.create(input)?;
        self.resolved(&quest.id)
    }

    // ---- Writing ----

    #[cfg(any(feature = "ui-plane", test))]
    pub fn update(&mut self, id: &str, input: UpdateQuestInput) -> Result<QuestChange, String> {
        self.update_with_origin(id, input, None)
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn update_with_origin(
        &mut self,
        id: &str,
        input: UpdateQuestInput,
        origin_device_id: Option<&str>,
    ) -> Result<QuestChange, String> {
        let patch = QuestUpdatePatch {
            input,
            description: QuestFieldPatch::Unchanged,
            due: QuestFieldPatch::Unchanged,
            due_time: QuestFieldPatch::Unchanged,
            repeat_rule: QuestFieldPatch::Unchanged,
        };
        self.update_scoped(id, patch, EditScope::All, None, origin_device_id)
    }

    pub fn update_patch(&mut self, id: &str, patch: QuestUpdatePatch) -> Result<QuestChange, String> {
        self.update_scoped(id, patch, EditScope::All, None, None)
    }

    pub fn update_patch_with_origin(
        &mut self,
        id: &str,
        patch: QuestUpdatePatch,
        origin_device_id: Option<&str>,
    ) -> Result<QuestChange, String> {
        self.update_scoped(id, patch, EditScope::All, None, origin_device_id)
    }

    /// One quest change. For a repeating quest (ADR-0012): Complete or Abandon
    /// closes one occurrence (`period_key`, or the oldest open); "active" on a
    /// closed day opens it again; `EditScope::Day` writes the fields as that
    /// day's overrides; `EditScope::All` edits the quest; clearing the rule
    /// stops the repeat (D10).
    pub fn update_scoped(
        &mut self,
        id: &str,
        patch: QuestUpdatePatch,
        scope: EditScope,
        period_key: Option<&str>,
        origin_device_id: Option<&str>,
    ) -> Result<QuestChange, String> {
        validate_optional_metadata(patch.input.energy, patch.input.priority)?;
        let quest = self.repository.get(id)?;
        if !matches!(patch.description, QuestFieldPatch::Unchanged) || patch.input.description.is_some() {
            Self::reject_prose_patch_on_checklist(&quest)?;
        }
        let marks = self.repository.occurrences(id)?;
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
            self.repository.update_patch(id, rest)?;
            let quest = self.repository.get(id)?;
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
                let saved = self.repository.save_occurrence(&mark)?;
                self.finish_repeating(id, vec![saved], Vec::new(), false, Some(&period_key))
            }
            EditScope::All => {
                let mut patch = patch;
                // The quest of a repeating series stays active; its days close.
                patch.input.status = None;
                let (_, _) = self.repository.update_patch(id, patch)?;
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
        let (_, quest) = self.repository.update_patch(&previous.id, patch)?;
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
                self.repository.conn,
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
                self.repository.conn,
                &quest.id,
                "completed_snapshot",
                &format!("Checklist at completion ({period_key}): {done}/{total} checked"),
                origin_device_id,
            );
        }
        self.repository.save_occurrence(&mark)
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
            let saved = self.repository.save_occurrence(&mark)?;
            self.finish_repeating(&id, vec![saved], Vec::new(), false, None)
        } else {
            self.repository.delete_occurrence(&id, period_key)?;
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
        self.repository.update_patch(&quest.id, patch)?;
        self.finish_repeating(&quest.id, Vec::new(), Vec::new(), true, None)
    }

    /// "Complete through today" (ADR-0012 D7): closes every open occurrence
    /// dated today or before, today's included.
    pub fn complete_through_today(&mut self, id: &str, origin_device_id: Option<&str>) -> Result<QuestChange, String> {
        let quest = self.repository.get(id)?;
        let marks = self.repository.occurrences(id)?;
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
        let stored = self.repository.get(id)?;
        let marks = self.repository.occurrences(id)?;
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
        let existing = self.repository.external_quest_checklist_state(&quest.id)?;
        let mut persisted = quest.clone();
        persisted.checklist_base = match existing {
            Some((false, _)) if quest.is_checklist => quest.description.clone(),
            Some((_, base)) => base,
            None if quest.is_checklist => quest.description.clone(),
            None => None,
        };
        self.repository.upsert_external_quest(&persisted)
    }

    /// A mark from a peer. A day's checklist copy merges with the local copy
    /// (the 3-way merge, D5); the other fields: the newer edit wins (the
    /// caller already compared the events).
    pub fn sync_upsert_occurrence(&mut self, incoming: &QuestOccurrence) -> Result<(), String> {
        let local = self.repository.find_occurrence(&incoming.quest_id, &incoming.period_key)?;
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
        self.repository.save_occurrence(&merged).map(|_| ())
    }

    pub fn sync_delete_occurrence(&mut self, quest_id: &str, period_key: &str) -> Result<(), String> {
        self.repository.delete_occurrence(quest_id, period_key)
    }

    /// Emits an upsert for each repeating quest and occurrence mark that has
    /// no sync event yet (the rows a migration wrote, ADR-0012 D11).
    #[cfg(any(feature = "ui-plane", test))]
    pub fn emit_missing_sync_events(&mut self, device_id: &str) -> Result<(), String> {
        let quests = self.repository.load_all()?;
        let marks = self.repository.all_occurrences()?;
        let space_by_quest: HashMap<&str, &str> =
            quests.iter().map(|quest| (quest.id.as_str(), quest.space_id.as_str())).collect();
        let conn = &mut *self.repository.conn;
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

    pub fn sync_quest_is_checklist(&mut self, id: &str) -> Result<bool, String> {
        self.repository.external_quest_is_checklist(id)
    }

    pub fn sync_merge_quest_checklist(
        &mut self,
        id: &str,
        incoming_description: Option<&str>,
    ) -> Result<(Option<String>, bool), String> {
        let local = self.repository.find_external_quest(id)?;
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
            self.repository.update_external_quest_checklist_merge(
                id,
                &merged,
                &next_checklist_base,
            )?;
        } else if base_advanced {
            self.repository
                .update_external_quest_checklist_base(id, &next_checklist_base)?;
        }

        Ok((
            merged.clone(),
            merged.as_deref() != incoming_description || base_advanced,
        ))
    }

    pub fn sync_move_quests_between_spaces(
        &mut self,
        local_space_id: &str,
        remote_space_id: &str,
    ) -> Result<(), String> {
        self.repository
            .move_quests_between_spaces(local_space_id, remote_space_id)
    }

    pub fn import_upsert_quest(&mut self, quest: &Quest) -> Result<(), String> {
        self.sync_upsert_quest(quest)
    }

    pub fn import_find_quest(&mut self, id: &str) -> Result<Option<Quest>, String> {
        self.repository.find_external_quest(id)
    }

    pub fn sync_delete_quest(&mut self, id: &str) -> Result<(), String> {
        self.repository.delete_external_quest(id)
    }

    /// Deletes the quest; for a repeating quest every occurrence and mark.
    pub fn delete(&mut self, id: &str) -> Result<Quest, String> {
        self.repository.delete(id)
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
        let quest = self.repository.get(quest_id)?;
        Self::ensure_checklist_quest(&quest)?;
        let marks = self.repository.occurrences(quest_id)?;
        let Some(repeating) = RepeatingQuest::new(&quest, &marks) else {
            let updated = edit(quest.description.as_deref().unwrap_or(""))?;
            let quest = self.repository.set_checklist_description(quest_id, &updated)?;
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
                let saved = self.repository.save_occurrence(&mark)?;
                self.finish_repeating(quest_id, vec![saved], Vec::new(), false, Some(&period_key))
            }
            EditScope::All => {
                let template = quest.description.clone().unwrap_or_default();
                let updated = edit(&checklist::reset_unchecked(&template))?;
                self.repository.set_checklist_description(quest_id, &updated)?;
                // The open day copies take the template edit (D5).
                let mut merged_days = Vec::new();
                for mark in marks.iter().filter(|mark| !mark.is_closed()) {
                    let Some(local) = mark.description.as_deref() else { continue };
                    let (description, _) = checklist::merge_3way(mark.checklist_base.as_deref(), local, &updated);
                    let mut mark = mark.clone();
                    mark.description = Some(description);
                    mark.checklist_base = Some(updated.clone());
                    mark.updated_at = utc_now();
                    merged_days.push(self.repository.save_occurrence(&mark)?);
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
            self.repository.conn,
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
            self.repository.conn,
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
            self.repository.conn,
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
            self.repository.conn,
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
            .load(self.repository.conn)
            .map_err(|e| e.to_string())
    }
}

/// Builds an `UpdateQuestInput` with only the fields that are set.
#[derive(Default)]
struct UpdateQuestInputBuilder {
    status: Option<String>,
    repeat_rule: Option<String>,
}

impl UpdateQuestInputBuilder {
    fn status(mut self, status: &str) -> Self {
        self.status = Some(status.to_string());
        self
    }

    fn repeat_rule(mut self, repeat_rule: String) -> Self {
        self.repeat_rule = Some(repeat_rule);
        self
    }

    fn build(self) -> crate::models::UpdateQuestInput {
        crate::models::UpdateQuestInput {
            space_id: None,
            title: None,
            description: None,
            status: self.status,
            energy: None,
            priority: None,
            due: None,
            due_time: None,
            repeat_rule: self.repeat_rule,
            order_rank: None,
            is_checklist: None,
        }
    }
}

/// Test helper: one update through the service; (quest as the UI shows it,
/// whether a restore should set Focus, ()).
#[cfg(test)]
pub fn update_quest_in_db(
    conn: &mut SqliteConnection,
    id: &str,
    input: crate::models::UpdateQuestInput,
) -> Result<(Quest, bool, ()), String> {
    let change = QuestService::new(conn).update(id, input)?;
    Ok((change.quest, change.restore_should_focus, ()))
}

fn log_checklist_activity(
    conn: &mut SqliteConnection,
    quest_id: &str,
    kind: &str,
    detail: &str,
    origin_device_id: Option<&str>,
) -> Result<(), String> {
    use crate::models::CreateChecklistActivityInput;

    let input = CreateChecklistActivityInput {
        quest_id: quest_id.to_string(),
        kind: kind.to_string(),
        detail: detail.to_string(),
        created_at: utc_now(),
        origin_device_id: origin_device_id.map(|s| s.to_string()),
    };
    diesel::insert_into(checklist_activity::table)
        .values(&input)
        .execute(conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn load_quests_for_list(
    conn: &mut SqliteConnection,
) -> Result<Vec<Quest>, diesel::result::Error> {
    let loaded: Vec<Quest> = quests::table.select(Quest::as_select()).load(conn)?;
    Ok(sort_quests_for_list(loaded))
}

fn sort_quests_for_list(mut loaded: Vec<Quest>) -> Vec<Quest> {
    let now = Utc::now();
    loaded.sort_by(|a, b| {
        let a_active = a.status == "active";
        let b_active = b.status == "active";
        if a_active != b_active {
            return b_active.cmp(&a_active);
        }

        if a_active {
            let a_overdue = is_overdue(a, &now);
            let b_overdue = is_overdue(b, &now);
            if a_overdue != b_overdue {
                return b_overdue.cmp(&a_overdue);
            }

            if (a.order_rank - b.order_rank).abs() > f64::EPSILON {
                return a
                    .order_rank
                    .partial_cmp(&b.order_rank)
                    .unwrap_or(std::cmp::Ordering::Equal);
            }

            if a.priority != b.priority {
                return b.priority.cmp(&a.priority);
            }
            return match (
                parse_utc_timestamp(&a.created_at),
                parse_utc_timestamp(&b.created_at),
            ) {
                (Some(a_created), Some(b_created)) if a_created != b_created => {
                    a_created.cmp(&b_created)
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                _ => a.created_at.cmp(&b.created_at),
            }
            .then_with(|| a.id.cmp(&b.id));
        }

        match (
            parse_utc_timestamp(&a.updated_at),
            parse_utc_timestamp(&b.updated_at),
        ) {
            (Some(a_updated), Some(b_updated)) if a_updated != b_updated => {
                b_updated.cmp(&a_updated)
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => b.updated_at.cmp(&a.updated_at),
        }
        .then_with(|| a.id.cmp(&b.id))
        .then_with(|| b.due.cmp(&a.due))
    });

    loaded
}

// ── Tauri commands ───────────────────────────────────────────────────────────

/// Keeps a quest's reminder in step with it: a one-time quest that is
/// active with a date; for a repeating quest its next open occurrence from
/// today (ADR-0012 D9).
#[cfg(any(feature = "ui-plane", test))]
fn refresh_reminder(conn: &mut SqliteConnection, app: &tauri::AppHandle, quest_id: &str) {
    let target = (|| -> Result<Option<Quest>, String> {
        let mut service = QuestService::new(conn);
        if service.is_repeating(quest_id)? {
            return service.reminder_occurrence(quest_id);
        }
        let quest = service.get(quest_id)?;
        Ok((quest.status == "active" && quest.due.is_some()).then_some(quest))
    })();
    let result = match target {
        Ok(Some(quest)) => reminder::upsert_reminder_for_quest(conn, app, &quest),
        Ok(None) => reminder::delete_reminder_for_quest(conn, app, quest_id),
        Err(e) => Err(e),
    };
    if let Err(e) = result {
        eprintln!("[bridge] reminder refresh failed for {quest_id}: {e}");
    }
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn get_quests(state: State<AppDbConnection>) -> Result<Vec<Quest>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    QuestService::new(&mut conn).list_for_ui()
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn create_quest(
    app: tauri::AppHandle,
    state: State<AppDbConnection>,
    space_sync: State<std::sync::Arc<SpaceSyncService>>,
    input: CreateQuestInput,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let quest = QuestService::new(&mut conn).create(input)?;
    emit_quest_sync_events(&mut conn, &space_sync.identity().device_id, &quest.space_id, &quest.id)?;
    refresh_reminder(&mut conn, &app, &quest.id);
    space_sync.wake();
    Ok(quest)
}

/// Complete a quest from a notification action. Handles its own DB locking and sync emission.
/// Logs errors rather than returning them — callers are notification action handlers. For a
/// repeating quest it closes the occurrence the reminder was for.
#[cfg(any(feature = "ui-plane", test))]
pub fn complete_quest_for_notification(app: &tauri::AppHandle, quest_id: &str) {
    let db = match app.try_state::<AppDbConnection>() {
        Some(s) => s,
        None => {
            eprintln!("[notification] complete: AppDbConnection not available");
            return;
        }
    };
    let space_sync = match app.try_state::<std::sync::Arc<SpaceSyncService>>() {
        Some(s) => s,
        None => {
            eprintln!("[notification] complete: SpaceSyncService not available");
            return;
        }
    };

    let mut conn = db.0.lock().unwrap();
    let device_id = space_sync.identity().device_id.clone();

    let quest: Quest = match QuestService::new(&mut conn).get(quest_id) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("[notification] complete: quest not found {quest_id}: {e}");
            return;
        }
    };
    let period_key = QuestService::new(&mut conn)
        .reminder_occurrence(quest_id)
        .ok()
        .flatten()
        .and_then(|day| day.due);

    let patch = QuestUpdatePatch {
        input: UpdateQuestInputBuilder::default().status(OCCURRENCE_COMPLETED).build(),
        description: QuestFieldPatch::Unchanged,
        due: QuestFieldPatch::Unchanged,
        due_time: QuestFieldPatch::Unchanged,
        repeat_rule: QuestFieldPatch::Unchanged,
    };
    match QuestService::new(&mut conn).update_scoped(
        quest_id,
        patch,
        EditScope::All,
        period_key.as_deref(),
        Some(&device_id),
    ) {
        Ok(change) => {
            refresh_reminder(&mut conn, app, quest_id);
            if let Err(e) = emit_change_sync_events(&mut conn, &device_id, &quest.space_id, &change) {
                eprintln!("[notification] complete: sync emit failed: {e}");
            }
            space_sync.wake();
        }
        Err(e) => eprintln!("[notification] complete: update failed for {quest_id}: {e}"),
    }
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn get_active_focus(state: State<AppDbConnection>) -> Result<Option<Quest>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    resolve_and_record_active_quest(&mut conn).map_err(|e| e.to_string())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn set_focus(state: State<AppDbConnection>, id: String) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    QuestService::new(&mut conn).set_focus(&id, "manual")
}

/// `input.scope` and `input.period_key` apply to a repeating quest only
/// (ADR-0012 D8).
#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn update_quest(
    app: tauri::AppHandle,
    state: State<AppDbConnection>,
    space_sync: State<std::sync::Arc<SpaceSyncService>>,
    id: String,
    input: UpdateQuestCommandInput,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let device_id = space_sync.identity().device_id.clone();
    let previous_space_id = QuestService::new(&mut conn).get(&id)?.space_id;
    let scope = EditScope::parse(input.scope.as_deref())?;
    let period_key = input.period_key.clone();

    let change = QuestService::new(&mut conn).update_scoped(
        &id,
        input.into_patch(),
        scope,
        period_key.as_deref(),
        Some(&device_id),
    )?;
    let mut quest = change.quest.clone();

    if change.restore_should_focus {
        append_focus_history(&mut conn, &quest.id, &quest.space_id, "restore")?;
        quest = record_focus_enter(&mut conn, &quest).map_err(|e| e.to_string())?;
    }

    refresh_reminder(&mut conn, &app, &id);
    emit_change_sync_events(&mut conn, &device_id, &previous_space_id, &change)?;
    space_sync.wake();

    Ok(quest)
}

/// "Complete through today" (ADR-0012 D7).
#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn complete_quest_through_today(
    app: tauri::AppHandle,
    state: State<AppDbConnection>,
    space_sync: State<std::sync::Arc<SpaceSyncService>>,
    id: String,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let device_id = space_sync.identity().device_id.clone();
    let space_id = QuestService::new(&mut conn).get(&id)?.space_id;
    let change = QuestService::new(&mut conn).complete_through_today(&id, Some(&device_id))?;
    refresh_reminder(&mut conn, &app, &id);
    emit_change_sync_events(&mut conn, &device_id, &space_id, &change)?;
    space_sync.wake();
    Ok(change.quest)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn delete_quest(
    app: tauri::AppHandle,
    state: State<AppDbConnection>,
    space_sync: State<std::sync::Arc<SpaceSyncService>>,
    id: String,
) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();

    let quest = QuestService::new(&mut conn).get(&id)?;

    // Cancel notifications before cascade delete removes the reminder rows
    if let Err(e) = reminder::delete_reminder_for_quest(&mut conn, &app, &id) {
        eprintln!("[bridge] delete_reminder on quest delete failed for {id}: {e}");
    }

    OutboxRepository::new(&mut conn).emit_sync_event(
        &space_sync.identity().device_id,
        "quest",
        &id,
        &quest.space_id,
        "delete",
        None,
    )?;

    QuestService::new(&mut conn).delete(&id)?;
    space_sync.wake();
    Ok(())
}

// ── Checklist commands (issue #128) ─────────────────────────────────────────

/// Syncs and returns a checklist change.
#[cfg(any(feature = "ui-plane", test))]
fn finish_checklist_command(
    conn: &mut SqliteConnection,
    space_sync: &SpaceSyncService,
    change: QuestChange,
) -> Result<Quest, String> {
    let device_id = space_sync.identity().device_id.clone();
    let space_id = QuestService::new(conn).get(&change.quest.id)?.space_id;
    emit_change_sync_events(conn, &device_id, &space_id, &change)?;
    space_sync.wake();
    Ok(change.quest)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn add_checklist_item(
    state: State<AppDbConnection>,
    space_sync: State<std::sync::Arc<SpaceSyncService>>,
    quest_id: String,
    text: String,
    scope: Option<String>,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let device_id = space_sync.identity().device_id.clone();
    let change = QuestService::new(&mut conn).add_checklist_item(
        &quest_id,
        &text,
        EditScope::parse(scope.as_deref())?,
        Some(&device_id),
    )?;
    finish_checklist_command(&mut conn, &space_sync, change)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn toggle_checklist_item(
    state: State<AppDbConnection>,
    space_sync: State<std::sync::Arc<SpaceSyncService>>,
    quest_id: String,
    item_id: String,
    checked: bool,
    period_key: Option<String>,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let change = QuestService::new(&mut conn).toggle_checklist_item(&quest_id, &item_id, checked, period_key.as_deref())?;
    finish_checklist_command(&mut conn, &space_sync, change)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn edit_checklist_item(
    state: State<AppDbConnection>,
    space_sync: State<std::sync::Arc<SpaceSyncService>>,
    quest_id: String,
    item_id: String,
    text: String,
    scope: Option<String>,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let device_id = space_sync.identity().device_id.clone();
    let change = QuestService::new(&mut conn).edit_checklist_item_text(
        &quest_id,
        &item_id,
        &text,
        EditScope::parse(scope.as_deref())?,
        Some(&device_id),
    )?;
    finish_checklist_command(&mut conn, &space_sync, change)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn remove_checklist_item(
    state: State<AppDbConnection>,
    space_sync: State<std::sync::Arc<SpaceSyncService>>,
    quest_id: String,
    item_id: String,
    scope: Option<String>,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let device_id = space_sync.identity().device_id.clone();
    let change = QuestService::new(&mut conn).remove_checklist_item(
        &quest_id,
        &item_id,
        EditScope::parse(scope.as_deref())?,
        Some(&device_id),
    )?;
    finish_checklist_command(&mut conn, &space_sync, change)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn reorder_checklist(
    state: State<AppDbConnection>,
    space_sync: State<std::sync::Arc<SpaceSyncService>>,
    quest_id: String,
    ordered_item_ids: Vec<String>,
    scope: Option<String>,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let change = QuestService::new(&mut conn).reorder_checklist(
        &quest_id,
        &ordered_item_ids,
        EditScope::parse(scope.as_deref())?,
    )?;
    finish_checklist_command(&mut conn, &space_sync, change)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn get_checklist_activity(
    state: State<AppDbConnection>,
    quest_id: String,
) -> Result<Vec<crate::models::ChecklistActivity>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    QuestService::new(&mut conn).get_checklist_activity(&quest_id)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quest_service_metadata_validation_rejects_values_outside_persistence_domain() {
        assert!(validate_metadata(1, 3).is_ok());
        assert_eq!(
            validate_metadata(0, 2).unwrap_err(),
            "invalid stored energy 0; expected 1, 2, or 3"
        );
        assert_eq!(
            validate_metadata(2, 4).unwrap_err(),
            "invalid stored priority 4; expected 1, 2, or 3"
        );
    }
    use crate::models::{clamp_order_rank, QuestFieldPatch};
    use crate::schema::sync_outbox;
    use crate::services::db::open_db_at_path;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        std::env::temp_dir().join(format!("fini-{label}-{unique}.db"))
    }

    fn is_uuid_like(value: &str) -> bool {
        value.len() == 36
            && value.as_bytes()[8] == b'-'
            && value.as_bytes()[13] == b'-'
            && value.as_bytes()[18] == b'-'
            && value.as_bytes()[23] == b'-'
    }

    fn insert_active_quest(
        conn: &mut SqliteConnection,
        title: &str,
        priority: i64,
        created_at: &str,
        due: Option<&str>,
        due_time: Option<&str>,
    ) -> String {
        use crate::schema::quests;
        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq(title),
                quests::status.eq("active"),
                quests::priority.eq(priority),
                quests::due.eq(due.map(str::to_string)),
                quests::due_time.eq(due_time.map(str::to_string)),
                quests::created_at.eq(created_at),
                quests::updated_at.eq(created_at),
            ))
            .execute(conn)
            .expect("insert test active quest");

        quests::table
            .filter(quests::title.eq(title))
            .select(quests::id)
            .first(conn)
            .expect("load inserted quest id")
    }

    fn status_patch(status: &str) -> UpdateQuestInput {
        UpdateQuestInput {
            space_id: None,
            title: None,
            description: None,
            status: Some(status.to_string()),
            energy: None,
            priority: None,
            due: None,
            due_time: None,
            repeat_rule: None,
            order_rank: None,
            is_checklist: None,
        }
    }

    fn due_patch(due: &str) -> UpdateQuestInput {
        UpdateQuestInput {
            space_id: None,
            title: None,
            description: None,
            status: None,
            energy: None,
            priority: None,
            due: Some(due.to_string()),
            due_time: None,
            repeat_rule: None,
            order_rank: None,
            is_checklist: None,
        }
    }

    fn space_patch(space_id: &str) -> UpdateQuestInput {
        UpdateQuestInput {
            space_id: Some(space_id.to_string()),
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
        }
    }

    #[test]
    fn nullable_patch_clears_each_quest_field_and_omission_preserves_values() {
        let db_path = temp_db_path("quest-nullable-patch");
        let mut conn = open_db_at_path(&db_path);
        let id = insert_active_quest(
            &mut conn,
            "nullable patch",
            1,
            "2026-03-28T10:00:00Z",
            Some("2026-04-01"),
            Some("10:30"),
        );
        diesel::update(quests::table.find(&id))
            .set((
                quests::description.eq(Some("keep or clear")),
                quests::repeat_rule.eq(Some(r#"{"preset":"weekly"}"#)),
            ))
            .execute(&mut conn)
            .expect("seed nullable fields");

        let patch = QuestUpdatePatch {
            input: status_patch("active"),
            description: QuestFieldPatch::Clear,
            due: QuestFieldPatch::Clear,
            due_time: QuestFieldPatch::Clear,
            repeat_rule: QuestFieldPatch::Clear,
        };
        let cleared = QuestService::new(&mut conn)
            .update_patch(&id, patch)
            .expect("clear nullable fields");
        assert_eq!(cleared.quest.description, None);
        assert_eq!(cleared.quest.due, None);
        assert_eq!(cleared.quest.due_time, None);
        assert_eq!(cleared.quest.repeat_rule, None);

        diesel::update(quests::table.find(&id))
            .set((
                quests::description.eq(Some("preserved")),
                quests::due.eq(Some("2026-05-01")),
                quests::due_time.eq(Some("08:00")),
                quests::repeat_rule.eq(Some("weekly")),
            ))
            .execute(&mut conn)
            .expect("reseed nullable fields");
        let preserved = QuestService::new(&mut conn)
            .update_patch(&id, QuestUpdatePatch::unchanged(status_patch("active")))
            .expect("omit nullable fields");
        assert_eq!(preserved.quest.description.as_deref(), Some("preserved"));
        assert_eq!(preserved.quest.due.as_deref(), Some("2026-05-01"));
        assert_eq!(preserved.quest.due_time.as_deref(), Some("08:00"));
        assert_eq!(preserved.quest.repeat_rule.as_deref(), Some("weekly"));

        let empty = QuestService::new(&mut conn)
            .update_patch(
                &id,
                QuestUpdatePatch {
                    input: status_patch("active"),
                    description: QuestFieldPatch::Set(String::new()),
                    due: QuestFieldPatch::Unchanged,
                    due_time: QuestFieldPatch::Unchanged,
                    repeat_rule: QuestFieldPatch::Unchanged,
                },
            )
            .expect("set an empty nullable field");
        assert_eq!(empty.quest.description.as_deref(), Some(""));
        assert_eq!(empty.quest.due.as_deref(), Some("2026-05-01"));

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn set_focus_records_manual_history_touches_and_counts_first_entry() {
        let db_path = temp_db_path("set-focus-service");
        let mut conn = open_db_at_path(&db_path);
        let id = insert_active_quest(
            &mut conn,
            "focus through service",
            1,
            "2020-01-01T00:00:00Z",
            None,
            None,
        );

        let focused = QuestService::new(&mut conn)
            .set_focus(&id, "manual")
            .expect("set active quest as focus");

        assert_eq!(focused.id, id);
        assert_eq!(focused.focus_enter_count, 1);
        assert_ne!(focused.updated_at, "2020-01-01T00:00:00Z");
        let triggers: Vec<String> = focus_history::table
            .filter(focus_history::quest_id.eq(&id))
            .select(focus_history::trigger)
            .load(&mut conn)
            .expect("load focus history");
        assert_eq!(triggers, vec!["manual"]);

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn order_rank_is_clamped_to_signed_100() {
        assert_eq!(clamp_order_rank(150.0), 100.0);
        assert_eq!(clamp_order_rank(-150.0), -100.0);
        assert_eq!(clamp_order_rank(42.5), 42.5);
    }

    #[test]
    fn quests_space_id_rejects_null() {
        let db_path = temp_db_path("quests-space-id-rejects-null");
        let mut conn = open_db_at_path(&db_path);

        let insert_result = diesel::sql_query(
            "INSERT INTO quests (space_id, title) VALUES (NULL, 'nullable-space-id-should-fail')",
        )
        .execute(&mut conn);

        assert!(
            insert_result.is_err(),
            "quests.space_id must be non-null and reject NULL inserts"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn quests_space_id_defaults_to_personal() {
        let db_path = temp_db_path("quests-space-id-defaults-to-personal");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(quests::table)
            .values(quests::title.eq("defaults-to-personal"))
            .execute(&mut conn)
            .expect("insert without space_id should succeed with default");

        let rows: Vec<String> = quests::table
            .filter(quests::title.eq("defaults-to-personal"))
            .select(quests::space_id)
            .load(&mut conn)
            .expect("query inserted quest");

        assert_eq!(rows.len(), 1, "must insert exactly one quest row");
        assert_eq!(rows[0], "1", "default quest space_id must be 1");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn new_quest_ids_are_uuid_like_strings() {
        let db_path = temp_db_path("new-quest-ids-are-uuid-like-strings");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(quests::table)
            .values(quests::title.eq("uuid-shape-check"))
            .execute(&mut conn)
            .expect("insert quest to check id shape");

        let ids: Vec<String> = quests::table
            .filter(quests::title.eq("uuid-shape-check"))
            .select(quests::id)
            .load(&mut conn)
            .expect("load created quest id");

        assert_eq!(ids.len(), 1, "must have exactly one created quest");
        let id = &ids[0];
        assert!(is_uuid_like(id), "quest id must look like UUID");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn manual_set_focus_overrides_fallback_selection() {
        let db_path = temp_db_path("manual-set-main-overrides-fallback-selection");
        let mut conn = open_db_at_path(&db_path);

        let low_priority_id = insert_active_quest(
            &mut conn,
            "manual-main-low",
            1,
            "2026-03-01T10:00:00Z",
            None,
            None,
        );
        let high_priority_id = insert_active_quest(
            &mut conn,
            "manual-main-high",
            3,
            "2026-03-02T10:00:00Z",
            None,
            None,
        );

        let before = resolve_active_quest(&mut conn)
            .expect("resolve before set-main")
            .expect("must return active quest");
        assert_eq!(
            before.id, high_priority_id,
            "without overrides, fallback should pick higher priority"
        );

        // Insert a focus_history row for the low-priority quest (simulates manual Focus set)
        diesel::sql_query(format!(
            "INSERT INTO focus_history (quest_id, space_id, trigger, created_at) \
             VALUES ('{}', '1', 'manual', '2026-03-03T12:00:00Z')",
            low_priority_id
        ))
        .execute(&mut conn)
        .expect("insert manual focus_history row");

        let after = resolve_active_quest(&mut conn)
            .expect("resolve after set-main")
            .expect("must return active quest");
        assert_eq!(
            after.id, low_priority_id,
            "focus_history entry must override fallback ordering"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn reminder_preemption_unwinds_after_resolution() {
        let db_path = temp_db_path("reminder-preemption-unwinds-after-resolution");
        let mut conn = open_db_at_path(&db_path);

        let manual_id = insert_active_quest(
            &mut conn,
            "manual-main",
            2,
            "2026-03-01T10:00:00Z",
            None,
            None,
        );
        let reminder_id = insert_active_quest(
            &mut conn,
            "reminder-main",
            2,
            "2026-03-01T11:00:00Z",
            None,
            None,
        );

        // Manual focus set at 09:00
        diesel::sql_query(format!(
            "INSERT INTO focus_history (quest_id, space_id, trigger, created_at) \
             VALUES ('{}', '1', 'manual', '2026-03-05T09:00:00Z')",
            manual_id
        ))
        .execute(&mut conn)
        .expect("insert manual focus_history row");

        let before_preempt = resolve_active_quest(&mut conn)
            .expect("resolve before reminder")
            .expect("must return active quest");
        assert_eq!(
            before_preempt.id, manual_id,
            "manual focus_history entry should be active before reminder"
        );

        // Reminder fires at 09:30 — newer than manual, so it preempts
        diesel::sql_query(format!(
            "INSERT INTO focus_history (quest_id, space_id, trigger, created_at) \
             VALUES ('{}', '1', 'reminder', '2026-03-05T09:30:00Z')",
            reminder_id
        ))
        .execute(&mut conn)
        .expect("insert reminder focus_history row");

        let preempted = resolve_active_quest(&mut conn)
            .expect("resolve during reminder")
            .expect("must return active quest");
        assert_eq!(
            preempted.id, reminder_id,
            "newest focus_history entry (reminder) should preempt manual Focus"
        );

        // Resolve the reminder quest — it becomes inactive
        diesel::update(quests::table.find(&reminder_id))
            .set(quests::status.eq("completed"))
            .execute(&mut conn)
            .expect("resolve reminder quest by completion");

        // Resolver skips completed quest, finds manual entry for manual_id
        let unwound = resolve_active_quest(&mut conn)
            .expect("resolve after reminder completion")
            .expect("must return active quest");
        assert_eq!(
            unwound.id, manual_id,
            "after reminder quest resolves, Focus should unwind to previous valid active entry"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn due_reminder_competes_with_manual_focus_by_timestamp() {
        let db_path = temp_db_path("due-reminder-competes-with-manual-focus-by-timestamp");
        let mut conn = open_db_at_path(&db_path);

        let manual_id = insert_active_quest(
            &mut conn,
            "manual-focus-before-reminder",
            2,
            "2026-03-01T10:00:00Z",
            None,
            None,
        );
        let reminder_id = insert_active_quest(
            &mut conn,
            "future-reminder-focus",
            2,
            "2026-03-01T11:00:00Z",
            Some("2026-03-05"),
            Some("09:31"),
        );

        let reminder_quest: Quest = quests::table
            .find(&reminder_id)
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("load reminder quest");
        let fire_at = parse_due_reminder_fire_utc(&reminder_quest).expect("parse reminder fire");
        let manual_at = fire_at - chrono::Duration::minutes(1);
        let manual_at_str = manual_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        diesel::sql_query(format!(
            "INSERT INTO focus_history (quest_id, space_id, trigger, created_at) \
             VALUES ('{}', '1', 'manual', '{}')",
            manual_id, manual_at_str
        ))
        .execute(&mut conn)
        .expect("insert manual focus_history row");

        let before_due = resolve_active_quest_at(&mut conn, fire_at - chrono::Duration::seconds(1))
            .expect("resolve before due")
            .expect("must return active quest");
        assert_eq!(
            before_due.id, manual_id,
            "future reminder must not preempt manual Focus before its fire time"
        );

        let after_due = resolve_active_quest_at(&mut conn, fire_at)
            .expect("resolve after due")
            .expect("must return active quest");
        assert_eq!(
            after_due.id, reminder_id,
            "due reminder timestamp must preempt older manual Focus"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn newer_manual_focus_beats_already_due_reminder() {
        let db_path = temp_db_path("newer-manual-focus-beats-already-due-reminder");
        let mut conn = open_db_at_path(&db_path);

        let manual_id = insert_active_quest(
            &mut conn,
            "manual-focus-after-reminder",
            2,
            "2026-03-01T10:00:00Z",
            None,
            None,
        );
        let reminder_id = insert_active_quest(
            &mut conn,
            "already-due-reminder-focus",
            2,
            "2026-03-01T11:00:00Z",
            Some("2026-03-05"),
            Some("09:31"),
        );

        let reminder_quest: Quest = quests::table
            .find(&reminder_id)
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("load reminder quest");
        let fire_at = parse_due_reminder_fire_utc(&reminder_quest).expect("parse reminder fire");
        let manual_at = fire_at + chrono::Duration::minutes(1);
        let manual_at_str = manual_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        diesel::sql_query(format!(
            "INSERT INTO focus_history (quest_id, space_id, trigger, created_at) \
             VALUES ('{}', '1', 'manual', '{}')",
            manual_id, manual_at_str
        ))
        .execute(&mut conn)
        .expect("insert manual focus_history row");

        let winner = resolve_active_quest_at(&mut conn, manual_at + chrono::Duration::seconds(1))
            .expect("resolve after manual")
            .expect("must return active quest");
        assert_eq!(
            winner.id, manual_id,
            "newer manual Focus timestamp must beat an older due reminder timestamp"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn fallback_order_is_overdue_then_priority_then_oldest() {
        let db_path = temp_db_path("fallback-order-overdue-priority-oldest");
        let mut conn = open_db_at_path(&db_path);

        let _overdue_low = insert_active_quest(
            &mut conn,
            "overdue-low",
            1,
            "2026-03-01T08:00:00Z",
            Some("2000-01-01"),
            None,
        );
        let overdue_urgent = insert_active_quest(
            &mut conn,
            "overdue-urgent",
            3,
            "2026-03-02T08:00:00Z",
            Some("2000-01-02"),
            None,
        );
        let _future_urgent = insert_active_quest(
            &mut conn,
            "future-urgent",
            3,
            "2026-03-03T08:00:00Z",
            Some("2999-01-01"),
            None,
        );

        let first = resolve_active_quest(&mut conn)
            .expect("resolve with mixed overdue state")
            .expect("must return active quest");
        assert_eq!(
            first.id, overdue_urgent,
            "among overdue quests, higher priority should win"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn fallback_uses_rank_before_priority() {
        let db_path = temp_db_path("fallback-uses-rank-before-priority");
        let mut conn = open_db_at_path(&db_path);

        let lower_rank_lower_priority = insert_active_quest(
            &mut conn,
            "rank-first-low-priority",
            1,
            "2026-03-01T08:00:00Z",
            None,
            None,
        );
        let higher_rank_higher_priority = insert_active_quest(
            &mut conn,
            "rank-first-high-priority",
            3,
            "2026-03-01T09:00:00Z",
            None,
            None,
        );

        diesel::update(quests::table.find(&lower_rank_lower_priority))
            .set(quests::order_rank.eq(-50.0))
            .execute(&mut conn)
            .expect("set lower rank");
        diesel::update(quests::table.find(&higher_rank_higher_priority))
            .set(quests::order_rank.eq(50.0))
            .execute(&mut conn)
            .expect("set higher rank");

        let winner = resolve_active_quest(&mut conn)
            .expect("resolve active after rank updates")
            .expect("must return active quest");

        assert_eq!(
            winner.id, lower_rank_lower_priority,
            "rank ordering must beat priority when no overdue or focus overrides exist"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn fallback_uses_oldest_created_at_on_tie() {
        let db_path = temp_db_path("fallback-uses-oldest-created-at-on-tie");
        let mut conn = open_db_at_path(&db_path);

        let oldest = insert_active_quest(
            &mut conn,
            "tie-oldest",
            3,
            "2026-03-01T08:00:00Z",
            None,
            None,
        );
        let newest = insert_active_quest(
            &mut conn,
            "tie-newest",
            3,
            "2026-03-02T08:00:00Z",
            None,
            None,
        );

        let winner = resolve_active_quest(&mut conn)
            .expect("resolve tie on fallback")
            .expect("must return active quest");

        assert_eq!(
            winner.id, oldest,
            "with equal overdue and priority, oldest created_at should win"
        );
        assert_ne!(winner.id, newest);

        let _ = std::fs::remove_file(db_path);
    }


    #[test]
    fn update_quest_command_input_null_actually_clears_due_due_time_and_repeat_rule() {
        let db_path = temp_db_path("update-command-input-clears-fields");
        let mut conn = open_db_at_path(&db_path);

        let id = insert_active_quest(
            &mut conn,
            "Clearable reminder",
            2,
            "2026-04-06T10:00:00Z",
            Some("2026-04-10"),
            Some("09:00"),
        );
        diesel::update(quests::table.find(&id))
            .set(quests::repeat_rule.eq(Some(r#"{"preset":"weekly"}"#)))
            .execute(&mut conn)
            .expect("seed repeat_rule");

        let command_input: crate::models::UpdateQuestCommandInput = serde_json::from_str(
            r#"{"due":null,"due_time":null,"repeat_rule":null}"#,
        )
        .expect("clear payload must deserialize");
        QuestService::new(&mut conn)
            .update_patch_with_origin(&id, command_input.into_patch(), None)
            .expect("clearing update must succeed");

        let cleared: Quest = quests::table
            .find(&id)
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("reload cleared quest");
        assert_eq!(cleared.due, None, "explicit null must clear due");
        assert_eq!(cleared.due_time, None, "explicit null must clear due_time");
        assert_eq!(
            cleared.repeat_rule, None,
            "explicit null must clear repeat_rule"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn update_quest_command_input_omitted_fields_leave_due_and_repeat_rule_untouched() {
        let db_path = temp_db_path("update-command-input-preserves-omitted-fields");
        let mut conn = open_db_at_path(&db_path);

        let id = insert_active_quest(
            &mut conn,
            "Untouched reminder",
            2,
            "2026-04-06T10:00:00Z",
            Some("2026-04-10"),
            Some("09:00"),
        );
        diesel::update(quests::table.find(&id))
            .set(quests::repeat_rule.eq(Some(r#"{"preset":"weekly"}"#)))
            .execute(&mut conn)
            .expect("seed repeat_rule");

        let command_input: crate::models::UpdateQuestCommandInput =
            serde_json::from_str(r#"{"priority":"high"}"#)
                .expect("partial payload must deserialize");
        QuestService::new(&mut conn)
            .update_patch_with_origin(&id, command_input.into_patch(), None)
            .expect("partial update must succeed");

        let updated: Quest = quests::table
            .find(&id)
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("reload updated quest");
        assert_eq!(
            updated.priority,
            crate::models::PRIORITY_HIGH,
            "priority must be applied"
        );
        assert_eq!(
            updated.due.as_deref(),
            Some("2026-04-10"),
            "omitted due must be left unchanged"
        );
        assert_eq!(
            updated.due_time.as_deref(),
            Some("09:00"),
            "omitted due_time must be left unchanged"
        );
        assert_eq!(
            updated.repeat_rule.as_deref(),
            Some(r#"{"preset":"weekly"}"#),
            "omitted repeat_rule must be left unchanged"
        );

        let _ = std::fs::remove_file(db_path);
    }




    #[test]
    fn restoring_future_due_quest_does_not_set_focus() {
        let db_path = temp_db_path("restore-future-due-does-not-focus-main");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("Future restore"),
                quests::status.eq("completed"),
                quests::due.eq("2999-01-01"),
                quests::completed_at.eq("2026-03-01T09:05:00Z"),
                quests::created_at.eq("2026-03-01T09:00:00Z"),
                quests::updated_at.eq("2026-03-01T09:05:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert completed future-due quest");

        let id: String = quests::table
            .filter(quests::title.eq("Future restore"))
            .select(quests::id)
            .first(&mut conn)
            .expect("load inserted quest id");

        let (restored, restore_should_focus, _) =
            update_quest_in_db(&mut conn, &id, status_patch("active"))
                .expect("restore future-due quest");

        assert_eq!(restored.status, "active");
        assert!(
            !restore_should_focus,
            "future-due restore must not write a focus_history entry"
        );

        let focus_count: i64 = focus_history::table
            .filter(focus_history::quest_id.eq(&id))
            .count()
            .get_result(&mut conn)
            .unwrap();
        assert_eq!(
            focus_count, 0,
            "future-due restore must leave focus_history empty"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn restoring_past_due_quest_appends_focus_history() {
        let db_path = temp_db_path("restore-past-due-focuses-main");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("Past restore"),
                quests::status.eq("completed"),
                quests::due.eq("2000-01-01"),
                quests::created_at.eq("2026-03-01T09:00:00Z"),
                quests::updated_at.eq("2026-03-01T09:05:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert completed past-due quest");

        let id: String = quests::table
            .filter(quests::title.eq("Past restore"))
            .select(quests::id)
            .first(&mut conn)
            .expect("load inserted quest id");

        let (restored, restore_should_focus, _) =
            update_quest_in_db(&mut conn, &id, status_patch("active"))
                .expect("restore past-due quest");

        assert_eq!(restored.status, "active");
        assert!(
            restore_should_focus,
            "past-due restore must signal that a focus_history entry should be written"
        );

        let _ = std::fs::remove_file(db_path);
    }



    #[test]
    fn moving_quest_between_spaces_emits_delete_then_upsert_events() {
        let db_path = temp_db_path("moving-quest-between-spaces-emits-delete-then-upsert");
        let mut conn = open_db_at_path(&db_path);

        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("space-move-events"),
                quests::status.eq("active"),
                quests::created_at.eq("2026-04-01T00:00:00Z"),
                quests::updated_at.eq("2026-04-01T00:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert quest");

        let id: String = quests::table
            .filter(quests::title.eq("space-move-events"))
            .select(quests::id)
            .first(&mut conn)
            .expect("load quest id");

        let (updated, _, _) =
            update_quest_in_db(&mut conn, &id, space_patch("2")).expect("move quest to space 2");
        emit_quest_sync_events(&mut conn, "device-local", "1", &updated.id)
            .expect("emit sync events for space move");

        let rows: Vec<(String, String, String, Option<String>)> = sync_outbox::table
            .filter(sync_outbox::entity_type.eq("quest"))
            .filter(sync_outbox::entity_id.eq(&id))
            .order(sync_outbox::created_at.asc())
            .select((
                sync_outbox::op_type,
                sync_outbox::space_id,
                sync_outbox::updated_at,
                sync_outbox::payload,
            ))
            .load(&mut conn)
            .expect("load emitted outbox rows");

        assert_eq!(rows.len(), 2, "space move must emit delete + upsert");
        assert_eq!(rows[0].0, "delete");
        assert_eq!(rows[0].1, "1");
        assert!(rows[0].3.is_none(), "delete event payload must be empty");
        assert_eq!(rows[1].0, "upsert");
        assert_eq!(rows[1].1, "2");
        assert!(rows[1].3.is_some(), "upsert event payload must be present");
        assert!(
            rows[1].2 > rows[0].2,
            "upsert event must be newer than delete event"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn checklist_activity_is_emitted_with_checklist_sync() {
        let db_path = temp_db_path("checklist-activity-sync-outbox");
        let mut conn = open_db_at_path(&db_path);

        let quest = QuestService::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Pack".to_string(),
                description: Some("- [ ] headphones <!--k=a1-->".to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: None,
                due_time: None,
                repeat_rule: None,
                order_rank: None,
                is_checklist: true,
            })
            .expect("create checklist quest");

        log_checklist_activity(
            &mut conn,
            &quest.id,
            "added",
            "Added \"headphones\"",
            Some("device-local"),
        )
        .expect("log checklist activity");

        emit_quest_sync_events(&mut conn, "device-local", &quest.space_id, &quest.id)
            .expect("emit quest sync with checklist activity");

        let rows: Vec<(String, String, String, Option<String>)> = sync_outbox::table
            .order(sync_outbox::created_at.asc())
            .select((
                sync_outbox::entity_type,
                sync_outbox::entity_id,
                sync_outbox::op_type,
                sync_outbox::payload,
            ))
            .load(&mut conn)
            .expect("load sync outbox rows");

        assert_eq!(rows.len(), 2, "quest and activity events should be emitted");
        assert_eq!(rows[0].0, "quest");
        assert_eq!(rows[0].1, quest.id);
        assert_eq!(rows[0].2, "upsert");
        assert_eq!(rows[1].0, "checklist_activity");
        assert_eq!(rows[1].2, "upsert");
        let payload = rows[1].3.as_ref().expect("activity payload");
        let activity: crate::models::ChecklistActivity = serde_json::from_str(payload).unwrap();
        assert_eq!(activity.quest_id, quest.id);
        assert_eq!(activity.kind, "added");
        assert_eq!(activity.detail, "Added \"headphones\"");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn checklist_sync_does_not_reemit_existing_activity_on_toggle() {
        let db_path = temp_db_path("checklist-activity-sync-no-reemit");
        let mut conn = open_db_at_path(&db_path);

        let quest = QuestService::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Pack".to_string(),
                description: Some("- [ ] headphones <!--k=a1-->".to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: None,
                due_time: None,
                repeat_rule: None,
                order_rank: None,
                is_checklist: true,
            })
            .expect("create checklist quest");

        log_checklist_activity(
            &mut conn,
            &quest.id,
            "added",
            "Added \"headphones\"",
            Some("device-local"),
        )
        .expect("log checklist activity");

        emit_quest_sync_events(&mut conn, "device-local", &quest.space_id, &quest.id)
            .expect("emit initial quest sync with checklist activity");

        let toggled = QuestService::new(&mut conn)
            .toggle_checklist_item(&quest.id, "a1", true, None)
            .expect("toggle checklist item")
            .quest;
        emit_quest_sync_events(&mut conn, "device-local", &toggled.space_id, &toggled.id)
            .expect("emit toggle quest sync without duplicate activity");

        let rows: Vec<(String, String)> = sync_outbox::table
            .order(sync_outbox::created_at.asc())
            .select((sync_outbox::entity_type, sync_outbox::op_type))
            .load(&mut conn)
            .expect("load sync outbox rows");

        assert_eq!(
            rows.len(),
            3,
            "initial quest/activity plus toggle quest only"
        );
        assert_eq!(rows[0], ("quest".to_string(), "upsert".to_string()));
        assert_eq!(
            rows[1],
            ("checklist_activity".to_string(), "upsert".to_string())
        );
        assert_eq!(rows[2], ("quest".to_string(), "upsert".to_string()));

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn moving_checklist_to_new_space_reemits_activity_for_destination_space() {
        let db_path = temp_db_path("checklist-activity-sync-space-move-reemit");
        let mut conn = open_db_at_path(&db_path);

        let quest = QuestService::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Pack".to_string(),
                description: Some("- [ ] headphones <!--k=a1-->".to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: None,
                due_time: None,
                repeat_rule: None,
                order_rank: None,
                is_checklist: true,
            })
            .expect("create checklist quest");

        log_checklist_activity(
            &mut conn,
            &quest.id,
            "added",
            "Added \"headphones\"",
            Some("device-local"),
        )
        .expect("log checklist activity");

        emit_quest_sync_events(&mut conn, "device-local", &quest.space_id, &quest.id)
            .expect("emit initial quest sync with checklist activity");

        let (moved, _, _) = update_quest_in_db(&mut conn, &quest.id, space_patch("2"))
            .expect("move checklist to space 2");
        emit_quest_sync_events(&mut conn, "device-local", &quest.space_id, &moved.id)
            .expect("emit move sync with checklist activity in destination space");

        let rows: Vec<(String, String, String)> = sync_outbox::table
            .order(sync_outbox::created_at.asc())
            .select((
                sync_outbox::entity_type,
                sync_outbox::op_type,
                sync_outbox::space_id,
            ))
            .load(&mut conn)
            .expect("load sync outbox rows");

        assert_eq!(
            rows,
            vec![
                ("quest".to_string(), "upsert".to_string(), "1".to_string()),
                (
                    "checklist_activity".to_string(),
                    "upsert".to_string(),
                    "1".to_string()
                ),
                ("quest".to_string(), "delete".to_string(), "1".to_string()),
                ("quest".to_string(), "upsert".to_string(), "2".to_string()),
                (
                    "checklist_activity".to_string(),
                    "upsert".to_string(),
                    "2".to_string()
                ),
            ],
            "space move must re-emit checklist activity under the destination space"
        );

        let _ = std::fs::remove_file(db_path);
    }





    // ── Checklist (issue #128) ──────────────────────────────────────────────
    //
    // Checklist content lives in the existing `description` field, parsed/rendered as task-list
    // text when `is_checklist` is set — there is no dedicated checklist content column.

    fn create_active_quest_with_checklist(conn: &mut SqliteConnection, checklist: &str) -> Quest {
        let result = QuestService::new(conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Go to office".to_string(),
                description: Some(checklist.to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: None,
                due_time: None,
                repeat_rule: None,
                order_rank: None,
                is_checklist: true,
            })
            .expect("create quest with checklist");
        result
    }

    fn create_active_quest_with_prose(conn: &mut SqliteConnection, description: &str) -> Quest {
        let result = QuestService::new(conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Meeting notes".to_string(),
                description: Some(description.to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: None,
                due_time: None,
                repeat_rule: None,
                order_rank: None,
                is_checklist: false,
            })
            .expect("create quest with prose");
        result
    }

    #[test]
    fn checklist_item_crud_logs_activity_but_toggles_do_not() {
        let db_path = temp_db_path("checklist-crud");
        let mut conn = open_db_at_path(&db_path);

        let quest = create_active_quest_with_checklist(&mut conn, "");
        let mut service = QuestService::new(&mut conn);

        let quest = service
            .add_checklist_item(&quest.id, "headphones", EditScope::All, Some("dev-a"))
            .expect("add item")
            .quest;
        let item_id = checklist::parse(quest.description.as_deref().unwrap())[0]
            .id
            .clone();

        let quest = service
            .toggle_checklist_item(&quest.id, &item_id, true, None)
            .expect("toggle item")
            .quest;
        assert!(checklist::parse(quest.description.as_deref().unwrap())[0].checked);

        let quest = service
            .edit_checklist_item_text(&quest.id, &item_id, "over-ear headphones", EditScope::All, Some("dev-a"))
            .expect("edit item text")
            .quest;
        assert_eq!(
            checklist::parse(quest.description.as_deref().unwrap())[0].text,
            "over-ear headphones"
        );

        let quest = service
            .remove_checklist_item(&quest.id, &item_id, EditScope::All, Some("dev-a"))
            .expect("remove item")
            .quest;
        assert!(checklist::parse(quest.description.as_deref().unwrap()).is_empty());

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        let kinds: Vec<&str> = activity.iter().map(|a| a.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["added", "edited", "removed"],
            "toggling checked state must not write an activity row"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn generic_description_patch_is_rejected_on_a_checklist_quest() {
        let db_path = temp_db_path("checklist-reject-prose-patch");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[checklist::ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: true,
        }]);
        let quest = create_active_quest_with_checklist(&mut conn, &template);

        // `quest update --description "notes"` must not be able to silently overwrite the
        // checklist text with prose, which would lose every item's id and checked state.
        let mut service = QuestService::new(&mut conn);
        let result = service.update(
            &quest.id,
            UpdateQuestInput {
                description: Some("notes".to_string()),
                ..status_patch("active")
            },
        );
        assert!(
            result.is_err(),
            "a plain description patch must be rejected for a checklist quest"
        );

        let stored = service.get(&quest.id).expect("load checklist quest");
        assert_eq!(
            stored.description.as_deref(),
            Some(template.as_str()),
            "rejected patch must not touch the stored checklist"
        );

        // The nullable-patch path (CLI's `quest update --description`) must reject the same way.
        let patch_result = service.update_patch(
            &quest.id,
            QuestUpdatePatch {
                input: status_patch("active"),
                description: QuestFieldPatch::Set("notes".to_string()),
                due: QuestFieldPatch::Unchanged,
                due_time: QuestFieldPatch::Unchanged,
                repeat_rule: QuestFieldPatch::Unchanged,
            },
        );
        assert!(
            patch_result.is_err(),
            "the nullable-patch path must reject a description Set on a checklist quest too"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn toggling_missing_checklist_item_is_rejected_without_writing() {
        let db_path = temp_db_path("checklist-toggle-missing-item");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[
            checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            checklist::ChecklistItem {
                id: "a2".into(),
                text: "key fob".into(),
                checked: true,
            },
        ]);
        let quest = create_active_quest_with_checklist(&mut conn, &template);
        let original_description = quest.description.clone();
        let original_updated_at = quest.updated_at.clone();

        let result = QuestService::new(&mut conn).toggle_checklist_item(&quest.id, "missing", true, None);
        assert!(matches!(
            result,
            Err(ref error) if error == "checklist item not found"
        ));

        let mut service = QuestService::new(&mut conn);
        let stored = service.get(&quest.id).expect("load checklist quest");
        assert_eq!(stored.description, original_description);
        assert_eq!(stored.updated_at, original_updated_at);

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        assert!(activity.is_empty(), "rejected toggle must not log activity");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn editing_missing_checklist_item_text_is_rejected_without_writing() {
        let db_path = temp_db_path("checklist-edit-missing-item");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[checklist::ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: false,
        }]);
        let quest = create_active_quest_with_checklist(&mut conn, &template);
        let original_description = quest.description.clone();

        let result = QuestService::new(&mut conn).edit_checklist_item_text(
            &quest.id,
            "missing",
            "new text",
            EditScope::All,
            Some("dev-a"),
        );
        assert!(matches!(
            result,
            Err(ref error) if error == "checklist item not found"
        ));

        let mut service = QuestService::new(&mut conn);
        let stored = service.get(&quest.id).expect("load checklist quest");
        assert_eq!(
            stored.description, original_description,
            "rejected edit must not rewrite the checklist"
        );

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        assert!(
            activity.is_empty(),
            "rejected edit must not log a false 'Renamed item' activity"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn removing_missing_checklist_item_is_rejected_without_writing() {
        let db_path = temp_db_path("checklist-remove-missing-item");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[checklist::ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: false,
        }]);
        let quest = create_active_quest_with_checklist(&mut conn, &template);
        let original_description = quest.description.clone();

        let result =
            QuestService::new(&mut conn).remove_checklist_item(&quest.id, "missing", EditScope::All, Some("dev-a"));
        assert!(matches!(
            result,
            Err(ref error) if error == "checklist item not found"
        ));

        let mut service = QuestService::new(&mut conn);
        let stored = service.get(&quest.id).expect("load checklist quest");
        assert_eq!(
            stored.description, original_description,
            "rejected remove must not rewrite the checklist"
        );

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        assert!(
            activity.is_empty(),
            "rejected remove must not log a false 'Removed an item' activity"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn toggling_non_active_checklist_item_is_rejected_without_writing() {
        for status in ["completed", "abandoned"] {
            let db_path = temp_db_path(&format!("checklist-toggle-non-active-{status}"));
            let mut conn = open_db_at_path(&db_path);

            let template = checklist::serialize(&[checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            }]);
            let quest = create_active_quest_with_checklist(&mut conn, &template);
            let mut service = QuestService::new(&mut conn);
            service
                .update(&quest.id, status_patch(status))
                .expect("move quest out of active status");
            let before = service
                .get(&quest.id)
                .expect("load non-active checklist quest");
            let before_activity = service
                .get_checklist_activity(&quest.id)
                .expect("load activity before rejected toggle");

            let result = service.toggle_checklist_item(&quest.id, "a1", true, None);
            assert!(matches!(
                result,
                Err(ref error) if error == "checklist toggles are only available for active quests"
            ));

            let after = service
                .get(&quest.id)
                .expect("reload non-active checklist quest");
            assert_eq!(after.description, before.description);
            assert_eq!(after.updated_at, before.updated_at);

            let after_activity = service
                .get_checklist_activity(&quest.id)
                .expect("load activity after rejected toggle");
            assert_eq!(
                after_activity.len(),
                before_activity.len(),
                "rejected non-active toggle must not add audit rows"
            );

            let _ = std::fs::remove_file(db_path);
        }
    }

    #[test]
    fn checklist_mutations_reject_prose_quests_without_conversion() {
        let db_path = temp_db_path("checklist-reject-prose");
        let mut conn = open_db_at_path(&db_path);

        let quest = create_active_quest_with_prose(&mut conn, "meeting notes");
        let mut service = QuestService::new(&mut conn);

        for result in [
            service.add_checklist_item(&quest.id, "headphones", EditScope::All, Some("dev-a")).map(|_| ()),
            service.toggle_checklist_item(&quest.id, "a1", true, None).map(|_| ()),
            service.edit_checklist_item_text(&quest.id, "a1", "headphones", EditScope::All, Some("dev-a")).map(|_| ()),
            service.remove_checklist_item(&quest.id, "a1", EditScope::All, Some("dev-a")).map(|_| ()),
            service.reorder_checklist(&quest.id, &["a1".to_string()], EditScope::All).map(|_| ()),
        ] {
            assert!(matches!(
                result,
                Err(ref error) if error == "quest is not a checklist"
            ));
        }

        let stored = service.get(&quest.id).expect("load prose quest");
        assert!(
            !stored.is_checklist,
            "mutation must not convert prose quests"
        );
        assert_eq!(stored.description.as_deref(), Some("meeting notes"));

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        assert!(
            activity.is_empty(),
            "rejected mutations must not log activity"
        );

        let _ = std::fs::remove_file(db_path);
    }


    #[test]
    fn completing_quest_with_checklist_logs_completed_snapshot() {
        let db_path = temp_db_path("checklist-completion-snapshot");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[
            checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: true,
            },
            checklist::ChecklistItem {
                id: "a2".into(),
                text: "lunch".into(),
                checked: false,
            },
        ]);
        let quest = create_active_quest_with_checklist(&mut conn, &template);

        let mut service = QuestService::new(&mut conn);
        service
            .update_with_origin(&quest.id, status_patch("completed"), Some("dev-a"))
            .expect("complete quest");

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        let snapshot = activity
            .iter()
            .find(|a| a.kind == "completed_snapshot")
            .expect("completed_snapshot activity row must exist");
        assert_eq!(snapshot.detail, "Checklist at completion: 1/2 checked");
        assert_eq!(snapshot.origin_device_id.as_deref(), Some("dev-a"));

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn editing_checklist_after_completion_logs_post_completion_edit() {
        let db_path = temp_db_path("checklist-post-completion-edit");
        let mut conn = open_db_at_path(&db_path);

        let quest = create_active_quest_with_checklist(&mut conn, "");
        let mut service = QuestService::new(&mut conn);
        service
            .update(&quest.id, status_patch("completed"))
            .expect("complete quest");

        service
            .add_checklist_item(&quest.id, "late addition", EditScope::All, Some("dev-a"))
            .expect("add item after completion");

        let activity = service
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        assert!(
            activity.iter().any(|a| a.kind == "post_completion_edit"),
            "post-completion checklist edits must be recorded as historical activity, per #128"
        );
        assert!(
            !activity.iter().any(|a| a.kind == "added"),
            "post-completion edits use the distinct kind, not the normal 'added' kind"
        );

        let _ = std::fs::remove_file(db_path);
    }

    // ── Repeating quests (ADR-0012) ─────────────────────────────────────────

    fn day(offset_days: i64) -> String {
        period_key_of(Utc::now().date_naive() + chrono::Duration::days(offset_days))
    }

    fn create_repeating(conn: &mut SqliteConnection, due: &str, rule: &str, checklist: Option<&str>) -> Quest {
        QuestService::new(conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Gym".to_string(),
                description: checklist.map(str::to_string),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: Some(due.to_string()),
                due_time: Some("20:00".to_string()),
                repeat_rule: Some(rule.to_string()),
                order_rank: None,
                is_checklist: checklist.is_some(),
            })
            .expect("create repeating quest")
    }

    const DAILY: &str = r#"{"preset":"daily"}"#;

    fn marks(conn: &mut SqliteConnection, quest_id: &str) -> Vec<(String, Option<String>)> {
        use crate::schema::quest_occurrences;
        quest_occurrences::table
            .filter(quest_occurrences::quest_id.eq(quest_id))
            .order(quest_occurrences::period_key.asc())
            .select((quest_occurrences::period_key, quest_occurrences::status))
            .load(conn)
            .expect("load marks")
    }

    #[test]
    fn a_repeating_quest_stores_no_day_rows_until_a_day_is_closed() {
        let db_path = temp_db_path("repeating-no-rows");
        let mut conn = open_db_at_path(&db_path);
        let quest = create_repeating(&mut conn, &day(-2), DAILY, None);

        let listed = QuestService::new(&mut conn).list_for_ui().expect("list");
        assert_eq!(listed.len(), 1, "one row for the quest, its oldest open day");
        assert_eq!(listed[0].due.as_deref(), Some(day(-2).as_str()));
        assert!(marks(&mut conn, &quest.id).is_empty(), "nothing stored while nobody acts");

        let change = QuestService::new(&mut conn).update(&quest.id, status_patch("completed")).expect("complete");
        assert_eq!(marks(&mut conn, &quest.id), vec![(day(-2), Some("completed".to_string()))]);
        assert_eq!(change.quest.due.as_deref(), Some(day(-1).as_str()), "the next open day shows");
        assert_eq!(change.quest.status, "active");
        let quest_rows: i64 = quests::table.count().get_result(&mut conn).unwrap();
        assert_eq!(quest_rows, 1, "complete writes no next occurrence row");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn complete_through_today_closes_each_open_day_through_today() {
        let db_path = temp_db_path("repeating-complete-through-today");
        let mut conn = open_db_at_path(&db_path);
        let quest = create_repeating(&mut conn, &day(-3), DAILY, None);

        let change = QuestService::new(&mut conn).complete_through_today(&quest.id, None).expect("complete through today");
        let expected: Vec<_> = (-3..=0).map(|offset| (day(offset), Some("completed".to_string()))).collect();
        assert_eq!(marks(&mut conn, &quest.id), expected, "one completed mark per day, today's included");
        assert_eq!(change.occurrences.len(), 4);
        assert_eq!(change.quest.due.as_deref(), Some(day(1).as_str()));

        let again = QuestService::new(&mut conn).complete_through_today(&quest.id, None);
        assert!(again.is_err(), "nothing is left to close through today");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn a_repeating_day_comes_to_focus_at_its_time() {
        let db_path = temp_db_path("repeating-focus-future-day");
        let mut conn = open_db_at_path(&db_path);
        let quest = create_repeating(&mut conn, &day(-2), DAILY, None);
        QuestService::new(&mut conn).set_focus(&quest.id, "manual").expect("set focus");
        assert_eq!(
            resolve_active_quest(&mut conn).expect("resolve").map(|q| q.id),
            Some(quest.id.clone()),
            "a missed day keeps the quest on Focus"
        );

        QuestService::new(&mut conn).complete_through_today(&quest.id, None).expect("complete through today");
        assert_eq!(
            resolve_active_quest(&mut conn).expect("resolve").map(|q| q.id),
            None,
            "the next day is tomorrow at 20:00; it has not come yet"
        );

        let comes_at = crate::services::due_time::compute_fire_utc(&day(1), Some("20:00")).expect("moment");
        assert_eq!(
            resolve_active_quest_at(&mut conn, comes_at - chrono::Duration::minutes(1)).expect("resolve").map(|q| q.id),
            None,
            "one minute before tomorrow's 20:00"
        );
        assert_eq!(
            resolve_active_quest_at(&mut conn, comes_at).expect("resolve").map(|q| q.id),
            Some(quest.id.clone()),
            "tomorrow at 20:00 the day comes to Focus"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn reopening_a_closed_day_removes_its_empty_mark() {
        let db_path = temp_db_path("repeating-reopen");
        let mut conn = open_db_at_path(&db_path);
        let quest = create_repeating(&mut conn, &day(-1), DAILY, None);
        QuestService::new(&mut conn).update(&quest.id, status_patch("completed")).expect("complete");

        let reopened = QuestService::new(&mut conn)
            .update_scoped(
                &quest.id,
                QuestUpdatePatch::unchanged(status_patch("active")),
                EditScope::All,
                Some(&day(-1)),
                None,
            )
            .expect("reopen the day");
        assert_eq!(reopened.removed_occurrences, vec![day(-1)]);
        assert!(marks(&mut conn, &quest.id).is_empty());
        assert_eq!(reopened.quest.due.as_deref(), Some(day(-1).as_str()), "the reopened day is the oldest open again");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn a_day_edit_overrides_one_day_and_an_all_days_edit_changes_every_day() {
        let db_path = temp_db_path("repeating-edit-scope");
        let mut conn = open_db_at_path(&db_path);
        let quest = create_repeating(&mut conn, &day(-1), DAILY, None);
        let rename = |title: &str| {
            QuestUpdatePatch::unchanged(UpdateQuestInput { title: Some(title.to_string()), ..status_patch("active") })
        };
        let mut no_status = rename("Gym (short)");
        no_status.input.status = None;

        QuestService::new(&mut conn)
            .update_scoped(&quest.id, no_status, EditScope::Day, None, None)
            .expect("this day");
        let mut service = QuestService::new(&mut conn);
        assert_eq!(service.get(&quest.id).unwrap().title, "Gym", "the quest keeps its title");
        assert_eq!(service.resolved(&quest.id).unwrap().title, "Gym (short)", "the day shows its own title");

        let mut all = rename("Run");
        all.input.status = None;
        service.update_scoped(&quest.id, all, EditScope::All, None, None).expect("all days");
        assert_eq!(service.get(&quest.id).unwrap().title, "Run");
        assert_eq!(service.resolved(&quest.id).unwrap().title, "Gym (short)", "the day keeps its override");
        service.update(&quest.id, status_patch("completed")).expect("complete the overridden day");
        assert_eq!(service.resolved(&quest.id).unwrap().title, "Run", "the next day takes the quest's title");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn a_tick_writes_a_day_copy_and_a_template_edit_reaches_it() {
        let db_path = temp_db_path("repeating-checklist-day-copy");
        let mut conn = open_db_at_path(&db_path);
        let template = checklist::serialize(&[
            checklist::ChecklistItem { id: "a1".into(), text: "headphones".into(), checked: false },
            checklist::ChecklistItem { id: "a2".into(), text: "charger".into(), checked: false },
        ]);
        let quest = create_repeating(&mut conn, &day(0), DAILY, Some(&template));

        let ticked = QuestService::new(&mut conn).toggle_checklist_item(&quest.id, "a1", true, None).expect("tick");
        assert!(checklist::parse(ticked.quest.description.as_deref().unwrap())[0].checked);
        let mut service = QuestService::new(&mut conn);
        assert_eq!(service.get(&quest.id).unwrap().description.as_deref(), Some(template.as_str()), "the template has no ticks");

        let added = service.add_checklist_item(&quest.id, "lunch", EditScope::All, None).expect("add to all days").quest;
        let items = checklist::parse(added.description.as_deref().unwrap());
        assert_eq!(items.len(), 3, "the open day copy took the template edit");
        assert!(items.iter().find(|it| it.id == "a1").unwrap().checked, "and kept its tick");
        assert!(!items.iter().find(|it| it.text == "lunch").unwrap().checked);

        service.update(&quest.id, status_patch("completed")).expect("complete the day");
        let next = service.resolved(&quest.id).unwrap();
        assert!(checklist::parse(next.description.as_deref().unwrap()).iter().all(|it| !it.checked), "the next day starts unticked");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn stopping_the_repeat_ends_the_rule_at_the_last_closed_day() {
        let db_path = temp_db_path("repeating-stop");
        let mut conn = open_db_at_path(&db_path);
        let quest = create_repeating(&mut conn, &day(-2), DAILY, None);
        QuestService::new(&mut conn).update(&quest.id, status_patch("completed")).expect("complete one day");

        let mut stop = QuestUpdatePatch::unchanged(status_patch("active"));
        stop.input.status = None;
        stop.repeat_rule = QuestFieldPatch::Clear;
        QuestService::new(&mut conn).update_patch(&quest.id, stop).expect("stop the repeat");

        let mut service = QuestService::new(&mut conn);
        let stored = service.get(&quest.id).unwrap();
        assert_eq!(stored.status, "completed", "the quest ends");
        assert!(stored.repeat_rule.as_deref().unwrap().contains(&day(-2)), "the rule ends at the last closed day");
        assert!(service.active_quests().unwrap().is_empty(), "no open day is left");
        assert_eq!(marks(&mut conn, &quest.id).len(), 1, "the closed day stays in History");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn a_rule_that_ends_after_n_has_no_open_day_after_n_completions() {
        let db_path = temp_db_path("repeating-end-after-n");
        let mut conn = open_db_at_path(&db_path);
        let rule = r#"{"preset":"daily","end":"after_n","end_after":2}"#;
        let quest = create_repeating(&mut conn, &day(-5), rule, None);

        QuestService::new(&mut conn).complete_through_today(&quest.id, None).expect("complete through today");
        assert_eq!(marks(&mut conn, &quest.id).len(), 2, "only the two days the rule allows close");
        assert!(QuestService::new(&mut conn).active_quests().unwrap().is_empty());

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn custom_rules_compute_their_dates() {
        let quest_with = |due: &str, rule: &str| Quest {
            due: Some(due.to_string()),
            repeat_rule: Some(rule.to_string()),
            ..test_quest_row()
        };
        let none: Vec<QuestOccurrence> = Vec::new();

        // Every 2 weeks on Monday and Wednesday, from Monday 2026-03-02.
        let quest = quest_with("2026-03-02", r#"{"preset":"custom","interval":2,"days_of_week":["Mo","We"]}"#);
        let repeating = RepeatingQuest::new(&quest, &none).unwrap();
        let dates: Vec<String> = repeating.dates().take(4).map(period_key_of).collect();
        assert_eq!(dates, ["2026-03-02", "2026-03-04", "2026-03-16", "2026-03-18"]);

        // Every 3 days, with no weekdays: the unit step.
        let quest = quest_with("2026-03-01", r#"{"preset":"custom","interval":3,"unit":"day"}"#);
        let repeating = RepeatingQuest::new(&quest, &none).unwrap();
        let dates: Vec<String> = repeating.dates().take(3).map(period_key_of).collect();
        assert_eq!(dates, ["2026-03-01", "2026-03-04", "2026-03-07"]);
    }

    fn test_quest_row() -> Quest {
        Quest {
            id: "q".to_string(),
            space_id: "1".to_string(),
            title: "Gym".to_string(),
            description: None,
            status: "active".to_string(),
            energy: crate::models::ENERGY_MEDIUM,
            priority: crate::models::PRIORITY_MEDIUM,
            due: None,
            due_time: None,
            repeat_rule: None,
            completed_at: None,
            order_rank: 0.0,
            focus_enter_count: 0,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            is_checklist: false,
            checklist_base: None,
        }
    }
}
