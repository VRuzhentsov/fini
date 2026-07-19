use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(any(feature = "ui-plane", test))]
use tauri::{Manager, State};

#[cfg(any(feature = "ui-plane", test))]
use crate::models::Space;
#[cfg(any(feature = "ui-plane", test))]
use crate::models::UpdateQuestInput;
use crate::models::{CreateFocusHistoryInput, CreateSeriesInput, Quest, QuestSeries};
use crate::models::{CreateQuestInput, QuestUpdatePatch};
use crate::repositories::quest::QuestRepository;
use crate::schema::{focus_history, settings};
#[cfg(test)]
use crate::schema::{quest_series, quests};
#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::utc_now;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::db::AppDbConnection;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::device_connection::DeviceConnectionState;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::reminder;
#[cfg(any(feature = "ui-plane", test))]
use crate::services::space_sync::outbox::{emit_sync_event, emit_sync_event_at};

// ── Repeat rule ──────────────────────────────────────────────────────────────

#[derive(Deserialize, Serialize, Clone, Debug)]
struct RepeatRule {
    preset: Option<String>,
    interval: Option<i64>,
    unit: Option<String>,
    days_of_week: Option<Vec<String>>,
    end: Option<String>,
    end_date: Option<String>,
    end_after: Option<i64>,
}

fn compute_next_due(current_due: &NaiveDate, rule: &RepeatRule) -> Option<NaiveDate> {
    use chrono::{Datelike, Months, Weekday};

    match rule.preset.as_deref() {
        Some("daily") => Some(*current_due + chrono::Duration::days(1)),
        Some("weekdays") => {
            let mut next = *current_due + chrono::Duration::days(1);
            while next.weekday() == Weekday::Sat || next.weekday() == Weekday::Sun {
                next += chrono::Duration::days(1);
            }
            Some(next)
        }
        Some("weekends") => {
            let mut next = *current_due + chrono::Duration::days(1);
            while next.weekday() != Weekday::Sat && next.weekday() != Weekday::Sun {
                next += chrono::Duration::days(1);
            }
            Some(next)
        }
        Some("weekly") => Some(*current_due + chrono::Duration::weeks(1)),
        Some("monthly") => current_due.checked_add_months(Months::new(1)),
        Some("yearly") => current_due.checked_add_months(Months::new(12)),
        Some("custom") => {
            let interval = rule.interval.unwrap_or(1).max(1);
            match rule.unit.as_deref() {
                Some("day") => Some(*current_due + chrono::Duration::days(interval)),
                Some("week") => Some(*current_due + chrono::Duration::weeks(interval)),
                Some("month") => current_due.checked_add_months(Months::new(interval as u32)),
                Some("year") => current_due.checked_add_months(Months::new(interval as u32 * 12)),
                _ => None,
            }
        }
        Some("none") | None => None,
        _ => None,
    }
}

fn is_series_end_reached(rule: &RepeatRule, next_due: &NaiveDate, completed_count: i64) -> bool {
    match rule.end.as_deref() {
        Some("on_date") => {
            if let Some(ref end_date_str) = rule.end_date {
                if let Ok(end_date) = NaiveDate::parse_from_str(end_date_str, "%Y-%m-%d") {
                    return *next_due > end_date;
                }
            }
            false
        }
        Some("after_n") => {
            if let Some(max) = rule.end_after {
                return completed_count >= max;
            }
            false
        }
        _ => false,
    }
}

/// Ensures a quest is linked to a series. If it has a repeat_rule but no series_id
/// (pre-migration data), creates the series on-the-fly and links the quest.
fn ensure_series(
    conn: &mut SqliteConnection,
    quest: &Quest,
) -> Result<Option<(String, RepeatRule)>, String> {
    let repeat_rule_str = match quest.repeat_rule.as_deref() {
        Some(r) if !r.is_empty() => r,
        _ => return Ok(None),
    };

    let rule: RepeatRule = match serde_json::from_str(repeat_rule_str) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };

    if let Some(ref sid) = quest.series_id {
        let active = QuestRepository::new(conn).is_series_active(sid)?;
        if !active {
            return Ok(None);
        }
        return Ok(Some((sid.clone(), rule)));
    }

    // Quest has repeat_rule but no series — backfill series on-the-fly
    let series_input = CreateSeriesInput {
        space_id: quest.space_id.clone(),
        title: quest.title.clone(),
        description: quest.description.clone(),
        repeat_rule: repeat_rule_str.to_string(),
        priority: quest.priority,
        energy: quest.energy.clone(),
    };

    let series = QuestRepository::new(conn).create_series(&series_input)?;

    let period_key = quest
        .due
        .as_deref()
        .unwrap_or(&quest.created_at[..10])
        .to_string();

    QuestRepository::new(conn).link_to_series(&quest.id, &series.id, &period_key)?;

    Ok(Some((series.id, rule)))
}

pub fn generate_next_occurrence(
    conn: &mut SqliteConnection,
    quest: &Quest,
) -> Result<Option<Quest>, String> {
    let (series_id, rule) = match ensure_series(conn, quest)? {
        Some(pair) => pair,
        None => return Ok(None),
    };

    let current_due = quest
        .due
        .as_deref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .unwrap_or_else(|| Utc::now().date_naive());

    let next_due = match compute_next_due(&current_due, &rule) {
        Some(d) => d,
        None => return Ok(None),
    };

    let completed_count =
        QuestRepository::new(conn).count_completed_series_occurrences(&series_id)?;

    if is_series_end_reached(&rule, &next_due, completed_count) {
        QuestRepository::new(conn).deactivate_series(&series_id)?;
        return Ok(None);
    }

    let period_key = next_due.format("%Y-%m-%d").to_string();

    if QuestRepository::new(conn).series_has_occurrence(&series_id, &period_key)? {
        return Ok(None);
    }
    QuestRepository::new(conn).create_next_occurrence(quest, &series_id, &period_key)
}

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

    Some((*fallback[0]).clone())
}

fn resolve_active_quest_at(
    conn: &mut SqliteConnection,
    now: DateTime<Utc>,
) -> Result<Option<Quest>, String> {
    let all_quests = QuestRepository::new(conn).load_all()?;
    let active_by_id: std::collections::HashMap<&str, &Quest> = all_quests
        .iter()
        .filter(|q| q.status == "active")
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

    if previous_focus_id.as_deref() != Some(quest.id.as_str()) {
        let quest = QuestRepository::new(conn).increment_focus_enter_count(&quest.id)?;
        save_active_focus_quest_id(conn, Some(&quest.id)).map_err(|error| error.to_string())?;
        return Ok(quest);
    }
    QuestRepository::new(conn).get(&quest.id)
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

#[cfg(any(feature = "ui-plane", test))]
fn emit_quest_sync_events(
    conn: &mut SqliteConnection,
    origin_device_id: &str,
    previous_space_id: &str,
    quest: &Quest,
) -> Result<(), String> {
    let payload = serde_json::to_string(quest).map_err(|e| e.to_string())?;

    if previous_space_id == quest.space_id {
        return emit_sync_event(
            conn,
            origin_device_id,
            "quest",
            &quest.id,
            &quest.space_id,
            "upsert",
            Some(payload),
        );
    }

    let delete_updated_at = utc_now();
    let upsert_updated_at = (Utc::now() + chrono::Duration::seconds(1))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    emit_sync_event_at(
        conn,
        origin_device_id,
        "quest",
        &quest.id,
        previous_space_id,
        "delete",
        None,
        delete_updated_at,
    )?;

    emit_sync_event_at(
        conn,
        origin_device_id,
        "quest",
        &quest.id,
        &quest.space_id,
        "upsert",
        Some(payload),
        upsert_updated_at,
    )?;

    Ok(())
}

pub(crate) struct CreateQuestResult {
    pub quest: Quest,
    pub series: Option<QuestSeries>,
}

pub(crate) struct UpdateQuestResult {
    pub quest: Quest,
    pub restore_should_focus: bool,
    pub next_occurrence: Option<Quest>,
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

    pub fn list_for_ui(&mut self) -> Result<Vec<Quest>, String> {
        self.repository.load_all().map(sort_quests_for_list)
    }

    pub fn get(&mut self, id: &str) -> Result<Quest, String> {
        self.repository.get(id)
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn get_with_space(&mut self, id: &str) -> Result<(Quest, Space), String> {
        self.repository.load_quest_and_space(id)
    }

    pub fn set_focus(&mut self, id: &str, trigger: &str) -> Result<Quest, String> {
        let quest = self
            .repository
            .get_active(id)?
            .ok_or_else(|| "cannot set Focus on non-active quest".to_string())?;
        self.repository.touch(id)?;
        append_focus_history(self.repository.conn, &quest.id, &quest.space_id, trigger)?;
        record_focus_enter(self.repository.conn, &quest)
    }

    pub fn create(&mut self, input: CreateQuestInput) -> Result<CreateQuestResult, String> {
        let repeats = input
            .repeat_rule
            .as_deref()
            .map(|rule| !rule.is_empty())
            .unwrap_or(false);
        if repeats {
            let (quest, series) = self.repository.create_series_and_quest(input)?;
            Ok(CreateQuestResult {
                quest,
                series: Some(series),
            })
        } else {
            Ok(CreateQuestResult {
                quest: self.repository.create(input)?,
                series: None,
            })
        }
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn update(
        &mut self,
        id: &str,
        input: UpdateQuestInput,
    ) -> Result<UpdateQuestResult, String> {
        let requested_status = input.status.clone();
        let (previous, quest) = self.repository.update(id, input)?;
        self.finish_update(previous, quest, requested_status)
    }

    pub fn update_patch(
        &mut self,
        id: &str,
        patch: QuestUpdatePatch,
    ) -> Result<UpdateQuestResult, String> {
        let requested_status = patch.input.status.clone();
        let (previous, quest) = self.repository.update_patch(id, patch)?;
        self.finish_update(previous, quest, requested_status)
    }

    fn finish_update(
        &mut self,
        previous: Quest,
        quest: Quest,
        requested_status: Option<String>,
    ) -> Result<UpdateQuestResult, String> {
        let restore_should_focus = previous.status != "active"
            && quest.status == "active"
            && should_set_focus_now_for_restore(quest.due.as_deref(), Utc::now());
        let next_occurrence = if matches!(
            requested_status.as_deref(),
            Some("completed") | Some("abandoned")
        ) && (quest.repeat_rule.is_some() || quest.series_id.is_some())
        {
            generate_next_occurrence(self.repository.conn, &quest).unwrap_or(None)
        } else {
            None
        };
        Ok(UpdateQuestResult {
            quest,
            restore_should_focus,
            next_occurrence,
        })
    }

    pub fn delete(&mut self, id: &str) -> Result<Quest, String> {
        self.repository.delete(id)
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn series_quest_ids(&mut self, series_id: &str) -> Result<Vec<String>, String> {
        self.repository
            .list_for_series(series_id)
            .map(|quests| quests.into_iter().map(|quest| quest.id).collect())
    }

    #[cfg(any(feature = "ui-plane", test))]
    pub fn delete_series_with_sync(
        &mut self,
        device_id: &str,
        series_id: &str,
    ) -> Result<(), diesel::result::Error> {
        self.repository.conn.transaction(|conn| {
            let (series_space_id, series_quests) = {
                let mut repository = QuestRepository::new(conn);
                let series_space_id = repository
                    .series_space_id(series_id)
                    .map_err(|_| diesel::result::Error::RollbackTransaction)?;
                let series_quests = repository
                    .list_for_series(series_id)
                    .map_err(|_| diesel::result::Error::RollbackTransaction)?;
                (series_space_id, series_quests)
            };

            for quest in &series_quests {
                emit_sync_event(
                    conn,
                    device_id,
                    "quest",
                    &quest.id,
                    &quest.space_id,
                    "delete",
                    None,
                )
                .map_err(|_| diesel::result::Error::RollbackTransaction)?;

                QuestRepository::new(conn)
                    .delete_series_quest(&quest.id)
                    .map_err(|_| diesel::result::Error::RollbackTransaction)?;
            }

            emit_sync_event(
                conn,
                device_id,
                "quest_series",
                series_id,
                &series_space_id,
                "delete",
                None,
            )
            .map_err(|_| diesel::result::Error::RollbackTransaction)?;

            QuestRepository::new(conn)
                .delete_series(series_id)
                .map_err(|_| diesel::result::Error::RollbackTransaction)?;
            Ok(())
        })
    }
}

#[cfg(test)]
pub fn update_quest_in_db(
    conn: &mut SqliteConnection,
    id: &str,
    input: UpdateQuestInput,
) -> Result<(Quest, bool, Option<Quest>), String> {
    let result = QuestService::new(conn).update(id, input)?;
    Ok((
        result.quest,
        result.restore_should_focus,
        result.next_occurrence,
    ))
}

fn compare_series_occurrence_order(a: &Quest, b: &Quest) -> std::cmp::Ordering {
    match (parse_due_deadline_utc(a), parse_due_deadline_utc(b)) {
        (Some(a_due), Some(b_due)) if a_due != b_due => a_due.cmp(&b_due),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        _ => match (
            parse_utc_timestamp(&a.created_at),
            parse_utc_timestamp(&b.created_at),
        ) {
            (Some(a_created), Some(b_created)) if a_created != b_created => {
                a_created.cmp(&b_created)
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => a.created_at.cmp(&b.created_at),
        },
    }
    .then_with(|| a.id.cmp(&b.id))
}

fn collapse_active_series_occurrences(loaded: Vec<Quest>) -> Vec<Quest> {
    let mut active_by_series: HashMap<String, Quest> = HashMap::new();
    let mut passthrough = Vec::with_capacity(loaded.len());

    for quest in loaded {
        if quest.status == "active" {
            if let Some(series_id) = quest.series_id.clone() {
                if let Some(current) = active_by_series.get_mut(&series_id) {
                    if compare_series_occurrence_order(&quest, current) == std::cmp::Ordering::Less
                    {
                        *current = quest;
                    }
                } else {
                    active_by_series.insert(series_id, quest);
                }
                continue;
            }
        }

        passthrough.push(quest);
    }

    passthrough.extend(active_by_series.into_values());
    passthrough
}

#[cfg(test)]
pub(crate) fn load_quests_for_list(
    conn: &mut SqliteConnection,
) -> Result<Vec<Quest>, diesel::result::Error> {
    let loaded: Vec<Quest> = quests::table.select(Quest::as_select()).load(conn)?;
    Ok(sort_quests_for_list(loaded))
}

fn sort_quests_for_list(loaded: Vec<Quest>) -> Vec<Quest> {
    let mut loaded = collapse_active_series_occurrences(loaded);

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
    });

    loaded
}

// ── Tauri commands ───────────────────────────────────────────────────────────

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
    device_connection: State<DeviceConnectionState>,
    input: CreateQuestInput,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let created = QuestService::new(&mut conn).create(input)?;

    if let Some(series) = &created.series {
        let series_payload = serde_json::to_string(&series).map_err(|e| e.to_string())?;
        emit_sync_event(
            &mut conn,
            &device_connection.identity.device_id,
            "quest_series",
            &series.id,
            &series.space_id,
            "upsert",
            Some(series_payload),
        )?;
    }

    let quest = created.quest;
    let payload = serde_json::to_string(&quest).map_err(|e| e.to_string())?;
    emit_sync_event(
        &mut conn,
        &device_connection.identity.device_id,
        "quest",
        &quest.id,
        &quest.space_id,
        "upsert",
        Some(payload),
    )?;

    if quest.status == "active" && quest.due.is_some() {
        if let Err(e) = reminder::upsert_reminder_for_quest(&mut conn, &app, &quest) {
            eprintln!(
                "[bridge] upsert_reminder on create failed for {}: {e}",
                quest.id
            );
        }
    }

    Ok(quest)
}

/// Complete a quest from a notification action. Handles its own DB locking and sync emission.
/// Logs errors rather than returning them — callers are notification action handlers.
#[cfg(any(feature = "ui-plane", test))]
pub fn complete_quest_for_notification(app: &tauri::AppHandle, quest_id: &str) {
    let db = match app.try_state::<AppDbConnection>() {
        Some(s) => s,
        None => {
            eprintln!("[notification] complete: AppDbConnection not available");
            return;
        }
    };
    let dc = match app.try_state::<DeviceConnectionState>() {
        Some(s) => s,
        None => {
            eprintln!("[notification] complete: DeviceConnectionState not available");
            return;
        }
    };

    let mut conn = db.0.lock().unwrap();

    let quest: Quest = match QuestService::new(&mut conn).get(quest_id) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("[notification] complete: quest not found {quest_id}: {e}");
            return;
        }
    };

    let space_id = quest.space_id.clone();
    let input = UpdateQuestInput {
        status: Some("completed".to_string()),
        space_id: None,
        title: None,
        description: None,
        energy: None,
        priority: None,
        pinned: None,
        due: None,
        due_time: None,
        repeat_rule: None,
        order_rank: None,
    };

    match QuestService::new(&mut conn).update(quest_id, input) {
        Ok(result) => {
            let updated_quest = result.quest;
            if let Err(e) = reminder::delete_reminder_for_quest(&mut conn, app, quest_id) {
                eprintln!("[notification] complete: delete_reminder failed: {e}");
            }
            if let Err(e) =
                emit_quest_sync_events(&mut conn, &dc.identity.device_id, &space_id, &updated_quest)
            {
                eprintln!("[notification] complete: sync emit failed: {e}");
            }
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

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn update_quest(
    app: tauri::AppHandle,
    state: State<AppDbConnection>,
    device_connection: State<DeviceConnectionState>,
    id: String,
    input: UpdateQuestInput,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let previous = QuestService::new(&mut conn).get(&id)?;
    let previous_status = previous.status.clone();
    let previous_space_id = previous.space_id.clone();

    let update_result = QuestService::new(&mut conn).update(&id, input)?;
    let mut quest = update_result.quest;

    if previous_status != "active" && quest.status == "active" && update_result.restore_should_focus
    {
        append_focus_history(&mut conn, &quest.id, &quest.space_id, "restore")?;
        quest = record_focus_enter(&mut conn, &quest).map_err(|e| e.to_string())?;
    }

    // Bridge: manage Reminder row based on resulting quest state
    let should_have_reminder = quest.status == "active" && quest.due.is_some();
    if should_have_reminder {
        if let Err(e) = reminder::upsert_reminder_for_quest(&mut conn, &app, &quest) {
            eprintln!("[bridge] upsert_reminder failed for {}: {e}", quest.id);
        }
    } else if let Err(e) = reminder::delete_reminder_for_quest(&mut conn, &app, &quest.id) {
        eprintln!("[bridge] delete_reminder failed for {}: {e}", quest.id);
    }

    // If a new occurrence was generated, create its reminder too
    if let Some(ref occ) = update_result.next_occurrence {
        if occ.due.is_some() {
            if let Err(e) = reminder::upsert_reminder_for_quest(&mut conn, &app, occ) {
                eprintln!("[bridge] upsert_reminder for occurrence failed: {e}");
            }
        }
    }

    emit_quest_sync_events(
        &mut conn,
        &device_connection.identity.device_id,
        &previous_space_id,
        &quest,
    )?;

    Ok(quest)
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn delete_quest(
    app: tauri::AppHandle,
    state: State<AppDbConnection>,
    device_connection: State<DeviceConnectionState>,
    id: String,
) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();

    let quest = QuestService::new(&mut conn).get(&id)?;

    // Cancel notifications before cascade delete removes the reminder rows
    if let Err(e) = reminder::delete_reminder_for_quest(&mut conn, &app, &id) {
        eprintln!("[bridge] delete_reminder on quest delete failed for {id}: {e}");
    }

    emit_sync_event(
        &mut conn,
        &device_connection.identity.device_id,
        "quest",
        &id,
        &quest.space_id,
        "delete",
        None,
    )?;

    QuestService::new(&mut conn).delete(&id)?;
    Ok(())
}

#[cfg(any(feature = "ui-plane", test))]
#[tauri::command]
pub fn delete_quest_series(
    app: tauri::AppHandle,
    state: State<AppDbConnection>,
    device_connection: State<DeviceConnectionState>,
    series_id: String,
) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();
    let origin_device_id = device_connection.identity.device_id.clone();

    // Cancel OS notifications before the DB transaction. Reminder cancellation is
    // best-effort: notification cancel is non-transactional and will not be undone
    // if the DB transaction below rolls back.
    let quest_ids = QuestService::new(&mut conn).series_quest_ids(&series_id)?;

    for quest_id in &quest_ids {
        if let Err(e) = reminder::delete_reminder_for_quest(&mut *conn, &app, quest_id) {
            eprintln!("[bridge] delete_reminder on series delete failed for {quest_id}: {e}");
        }
    }

    QuestService::new(&mut conn)
        .delete_series_with_sync(&origin_device_id, &series_id)
        .map_err(|e| e.to_string())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
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
            pinned: None,
            due: None,
            due_time: None,
            repeat_rule: None,
            order_rank: None,
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
            pinned: None,
            due: Some(due.to_string()),
            due_time: None,
            repeat_rule: None,
            order_rank: None,
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
            pinned: None,
            due: None,
            due_time: None,
            repeat_rule: None,
            order_rank: None,
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
            4,
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
            4,
            "2026-03-02T08:00:00Z",
            Some("2000-01-02"),
            None,
        );
        let _future_urgent = insert_active_quest(
            &mut conn,
            "future-urgent",
            4,
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
            4,
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
    fn recurring_quest_lifecycle_create_complete_generates_and_delete() {
        let db_path = temp_db_path("recurring-quest-lifecycle");
        let mut conn = open_db_at_path(&db_path);

        let repeat_rule = r#"{"preset":"weekly"}"#;
        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("Weekly chore"),
                quests::status.eq("active"),
                quests::due.eq("2026-04-01"),
                quests::repeat_rule.eq(repeat_rule),
                quests::created_at.eq("2026-03-29T10:00:00Z"),
                quests::updated_at.eq("2026-03-29T10:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert recurring quest");

        let first: Quest = quests::table
            .filter(quests::title.eq("Weekly chore"))
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("load first occurrence");

        assert_eq!(first.status, "active");
        assert_eq!(first.due.as_deref(), Some("2026-04-01"));
        assert!(first.series_id.is_none(), "no series yet before completion");

        diesel::update(quests::table.find(&first.id))
            .set((
                quests::status.eq("completed"),
                quests::completed_at.eq("2026-04-01T18:00:00Z"),
                quests::updated_at.eq("2026-04-01T18:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("complete first occurrence");

        let completed: Quest = quests::table
            .find(&first.id)
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("reload completed quest");

        let _ = generate_next_occurrence(&mut conn, &completed);

        let linked: Quest = quests::table
            .find(&first.id)
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("reload linked quest");
        assert!(
            linked.series_id.is_some(),
            "series must be backfilled on completion"
        );
        assert!(
            linked.period_key.is_some(),
            "period_key must be set on completion"
        );

        let series_id = linked.series_id.unwrap();

        let next: Quest = quests::table
            .filter(quests::series_id.eq(&series_id))
            .filter(quests::status.eq("active"))
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("next occurrence must exist");

        assert_eq!(next.title, "Weekly chore");
        assert_eq!(
            next.due.as_deref(),
            Some("2026-04-08"),
            "next weekly occurrence must be 7 days later"
        );
        assert_eq!(next.series_id.as_deref(), Some(series_id.as_str()));
        assert_eq!(next.period_key.as_deref(), Some("2026-04-08"));
        assert_eq!(
            next.repeat_rule.as_deref(),
            Some(repeat_rule),
            "repeat_rule must carry over"
        );

        diesel::delete(quests::table.find(&next.id))
            .execute(&mut conn)
            .expect("delete next occurrence");

        let remaining = quests::table
            .filter(quests::series_id.eq(&series_id))
            .filter(quests::status.eq("active"))
            .count()
            .get_result::<i64>(&mut conn)
            .expect("count remaining active");
        assert_eq!(remaining, 0, "no active occurrences after delete");

        let series_exists = quest_series::table
            .find(&series_id)
            .count()
            .get_result::<i64>(&mut conn)
            .expect("check series exists");
        assert_eq!(
            series_exists, 1,
            "series record must persist after deleting occurrence"
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
    fn restoring_occurrence_from_history_shows_single_active_series_item() {
        let db_path = temp_db_path("restore-occurrence-history-single-active-series-item");
        let mut conn = open_db_at_path(&db_path);

        let repeat_rule = r#"{"preset":"weekly"}"#;
        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("Weekly sync"),
                quests::status.eq("active"),
                quests::due.eq("2026-04-01"),
                quests::repeat_rule.eq(repeat_rule),
                quests::created_at.eq("2026-03-29T10:00:00Z"),
                quests::updated_at.eq("2026-03-29T10:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert recurring quest");

        let first: Quest = quests::table
            .filter(quests::title.eq("Weekly sync"))
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("load first occurrence");

        diesel::update(quests::table.find(&first.id))
            .set((
                quests::status.eq("completed"),
                quests::completed_at.eq("2026-04-01T18:00:00Z"),
                quests::updated_at.eq("2026-04-01T18:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("complete first occurrence");

        let completed: Quest = quests::table
            .find(&first.id)
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("reload completed first occurrence");

        let _ = generate_next_occurrence(&mut conn, &completed);

        let linked: Quest = quests::table
            .find(&first.id)
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("reload first occurrence with series linkage");
        let series_id = linked
            .series_id
            .clone()
            .expect("series must exist after first completion");

        diesel::update(quests::table.find(&linked.id))
            .set((
                quests::status.eq("active"),
                quests::completed_at.eq(Option::<String>::None),
                quests::updated_at.eq("2026-04-02T09:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("restore first occurrence from history");

        diesel::sql_query(format!(
            "INSERT INTO focus_history (quest_id, space_id, trigger, created_at) \
             VALUES ('{}', '1', 'restore', '2026-04-02T09:00:00Z')",
            linked.id
        ))
        .execute(&mut conn)
        .expect("insert restore focus_history row");

        let active_in_db = quests::table
            .filter(quests::series_id.eq(&series_id))
            .filter(quests::status.eq("active"))
            .count()
            .get_result::<i64>(&mut conn)
            .expect("count active occurrences in series");
        assert_eq!(
            active_in_db, 2,
            "restore flow leaves multiple unresolved series occurrences in storage"
        );

        let visible =
            load_quests_for_list(&mut conn).expect("load quests for active/history lists");
        let visible_series_active: Vec<&Quest> = visible
            .iter()
            .filter(|quest| {
                quest.status == "active" && quest.series_id.as_deref() == Some(series_id.as_str())
            })
            .collect();

        assert_eq!(
            visible_series_active.len(),
            1,
            "active quest lists must show only one unresolved occurrence per series"
        );
        assert_eq!(
            visible_series_active[0].id, linked.id,
            "restored occurrence should be shown instead of later generated one"
        );
        assert_eq!(
            visible_series_active[0].due.as_deref(),
            Some("2026-04-01"),
            "restored earliest unresolved occurrence must be surfaced"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn updating_due_on_series_occurrence_syncs_period_key() {
        let db_path = temp_db_path("updating-due-syncs-period-key");
        let mut conn = open_db_at_path(&db_path);

        let repeat_rule = r#"{"preset":"weekly"}"#;
        let series = diesel::insert_into(quest_series::table)
            .values(&CreateSeriesInput {
                space_id: "1".to_string(),
                title: "Weekly sync".to_string(),
                description: None,
                repeat_rule: repeat_rule.to_string(),
                priority: 1,
                energy: "medium".to_string(),
            })
            .returning(QuestSeries::as_returning())
            .get_result::<QuestSeries>(&mut conn)
            .expect("insert series");

        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("Weekly sync"),
                quests::status.eq("active"),
                quests::due.eq("2026-03-28"),
                quests::repeat_rule.eq(repeat_rule),
                quests::series_id.eq(&series.id),
                quests::period_key.eq("2026-03-28"),
                quests::created_at.eq("2026-03-28T10:00:00Z"),
                quests::updated_at.eq("2026-03-28T10:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert series occurrence");

        let id: String = quests::table
            .filter(quests::title.eq("Weekly sync"))
            .select(quests::id)
            .first(&mut conn)
            .expect("load inserted occurrence id");

        let (updated, _, _) = update_quest_in_db(&mut conn, &id, due_patch("2026-03-29"))
            .expect("update due on recurring occurrence");

        assert_eq!(updated.due.as_deref(), Some("2026-03-29"));
        assert_eq!(
            updated.period_key.as_deref(),
            Some("2026-03-29"),
            "period_key must follow due when editing recurring occurrence date"
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
        emit_quest_sync_events(&mut conn, "device-local", "1", &updated)
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
    fn updating_due_to_existing_series_period_key_is_rejected() {
        let db_path = temp_db_path("updating-due-conflicting-period-key-is-rejected");
        let mut conn = open_db_at_path(&db_path);

        let repeat_rule = r#"{"preset":"weekly"}"#;
        let series = diesel::insert_into(quest_series::table)
            .values(&CreateSeriesInput {
                space_id: "1".to_string(),
                title: "Laundry".to_string(),
                description: None,
                repeat_rule: repeat_rule.to_string(),
                priority: 1,
                energy: "medium".to_string(),
            })
            .returning(QuestSeries::as_returning())
            .get_result::<QuestSeries>(&mut conn)
            .expect("insert series");

        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("Laundry"),
                quests::status.eq("active"),
                quests::due.eq("2026-03-28"),
                quests::repeat_rule.eq(repeat_rule),
                quests::series_id.eq(&series.id),
                quests::period_key.eq("2026-03-28"),
                quests::created_at.eq("2026-03-28T10:00:00Z"),
                quests::updated_at.eq("2026-03-28T10:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert first occurrence");

        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("Laundry"),
                quests::status.eq("active"),
                quests::due.eq("2026-04-04"),
                quests::repeat_rule.eq(repeat_rule),
                quests::series_id.eq(&series.id),
                quests::period_key.eq("2026-04-04"),
                quests::created_at.eq("2026-03-29T10:00:00Z"),
                quests::updated_at.eq("2026-03-29T10:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert second occurrence");

        let first_id: String = quests::table
            .filter(quests::series_id.eq(&series.id))
            .filter(quests::period_key.eq("2026-03-28"))
            .select(quests::id)
            .first(&mut conn)
            .expect("load first occurrence id");

        let err = match update_quest_in_db(&mut conn, &first_id, due_patch("2026-04-04")) {
            Ok(_) => panic!("conflicting period_key update must fail"),
            Err(err) => err,
        };
        assert!(
            err.contains("occurrence for this date already exists in the series"),
            "error should explain duplicate series date conflict"
        );

        let first: Quest = quests::table
            .find(&first_id)
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("reload first occurrence");
        assert_eq!(
            first.period_key.as_deref(),
            Some("2026-03-28"),
            "failed update must not mutate period_key"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn delete_quest_series_removes_all_children_and_series_row() {
        let db_path = temp_db_path("delete-quest-series-removes-all");
        let mut conn = open_db_at_path(&db_path);

        let repeat_rule = r#"{"preset":"daily"}"#;
        let series = diesel::insert_into(quest_series::table)
            .values(&CreateSeriesInput {
                space_id: "1".to_string(),
                title: "Daily standup".to_string(),
                description: None,
                repeat_rule: repeat_rule.to_string(),
                priority: 1,
                energy: "medium".to_string(),
            })
            .returning(QuestSeries::as_returning())
            .get_result::<QuestSeries>(&mut conn)
            .expect("insert series");

        for (status, period) in [
            ("active", "2026-05-15"),
            ("completed", "2026-05-14"),
            ("abandoned", "2026-05-13"),
        ] {
            diesel::insert_into(quests::table)
                .values((
                    quests::space_id.eq("1"),
                    quests::title.eq("Daily standup"),
                    quests::status.eq(status),
                    quests::series_id.eq(&series.id),
                    quests::period_key.eq(period),
                    quests::created_at.eq("2026-05-13T10:00:00Z"),
                    quests::updated_at.eq("2026-05-13T10:00:00Z"),
                ))
                .execute(&mut conn)
                .expect("insert occurrence");
        }

        let count_before: i64 = quests::table
            .filter(quests::series_id.eq(&series.id))
            .count()
            .get_result(&mut conn)
            .expect("count before");
        assert_eq!(count_before, 3, "expected 3 occurrences before delete");

        QuestService::new(&mut conn)
            .delete_series_with_sync("device-test", &series.id)
            .expect("delete series");

        let count_after: i64 = quests::table
            .filter(quests::series_id.eq(&series.id))
            .count()
            .get_result(&mut conn)
            .expect("count after");
        assert_eq!(count_after, 0, "all occurrence quests must be gone");

        let series_count: i64 = quest_series::table
            .filter(quest_series::id.eq(&series.id))
            .count()
            .get_result(&mut conn)
            .expect("count series");
        assert_eq!(series_count, 0, "quest_series row must be gone");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn delete_quest_series_emits_sync_events_for_each_quest_and_series() {
        let db_path = temp_db_path("delete-quest-series-sync-events");
        let mut conn = open_db_at_path(&db_path);

        let repeat_rule = r#"{"preset":"daily"}"#;
        let series = diesel::insert_into(quest_series::table)
            .values(&CreateSeriesInput {
                space_id: "1".to_string(),
                title: "Daily sync".to_string(),
                description: None,
                repeat_rule: repeat_rule.to_string(),
                priority: 1,
                energy: "medium".to_string(),
            })
            .returning(QuestSeries::as_returning())
            .get_result::<QuestSeries>(&mut conn)
            .expect("insert series");

        for (status, period) in [
            ("active", "2026-05-15"),
            ("completed", "2026-05-14"),
            ("abandoned", "2026-05-13"),
        ] {
            diesel::insert_into(quests::table)
                .values((
                    quests::space_id.eq("1"),
                    quests::title.eq("Daily sync"),
                    quests::status.eq(status),
                    quests::series_id.eq(&series.id),
                    quests::period_key.eq(period),
                    quests::created_at.eq("2026-05-13T10:00:00Z"),
                    quests::updated_at.eq("2026-05-13T10:00:00Z"),
                ))
                .execute(&mut conn)
                .expect("insert occurrence");
        }

        QuestService::new(&mut conn)
            .delete_series_with_sync("device-test", &series.id)
            .expect("delete series");

        let events: Vec<(String, String)> = sync_outbox::table
            .select((sync_outbox::entity_type, sync_outbox::op_type))
            .load(&mut conn)
            .expect("load outbox events");

        let quest_deletes = events
            .iter()
            .filter(|(t, op)| t == "quest" && op == "delete")
            .count();
        let series_deletes = events
            .iter()
            .filter(|(t, op)| t == "quest_series" && op == "delete")
            .count();

        assert_eq!(
            quest_deletes, 3,
            "must emit one delete event per occurrence"
        );
        assert_eq!(
            series_deletes, 1,
            "must emit one delete event for quest_series"
        );

        let _ = std::fs::remove_file(db_path);
    }
}
