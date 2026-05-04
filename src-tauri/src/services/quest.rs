use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::State;

use crate::models::{
    clamp_order_rank, CreateFocusHistoryInput, CreateQuestInput, CreateSeriesInput, Quest,
    QuestSeries, UpdateQuestInput,
};
use crate::schema::{focus_history, quest_series, quests};
use crate::services::db::{utc_now, DbState};
use crate::services::device_connection::DeviceConnectionState;
use crate::services::{notification, reminder};
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
) -> Result<Option<(String, RepeatRule)>, diesel::result::Error> {
    let repeat_rule_str = match quest.repeat_rule.as_deref() {
        Some(r) if !r.is_empty() => r,
        _ => return Ok(None),
    };

    let rule: RepeatRule = match serde_json::from_str(repeat_rule_str) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };

    if let Some(ref sid) = quest.series_id {
        let active: bool = quest_series::table
            .find(sid)
            .select(quest_series::active)
            .first(conn)
            .unwrap_or(true);
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

    let series = diesel::insert_into(quest_series::table)
        .values(&series_input)
        .returning(QuestSeries::as_returning())
        .get_result::<QuestSeries>(conn)?;

    let period_key = quest
        .due
        .as_deref()
        .unwrap_or(&quest.created_at[..10])
        .to_string();

    diesel::update(quests::table.find(&quest.id))
        .set((
            quests::series_id.eq(&series.id),
            quests::period_key.eq(&period_key),
        ))
        .execute(conn)?;

    Ok(Some((series.id, rule)))
}

pub fn generate_next_occurrence(
    conn: &mut SqliteConnection,
    quest: &Quest,
) -> Result<Option<Quest>, diesel::result::Error> {
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

    let completed_count: i64 = quests::table
        .filter(quests::series_id.eq(&series_id))
        .filter(quests::status.ne("active"))
        .count()
        .get_result(conn)?;

    if is_series_end_reached(&rule, &next_due, completed_count) {
        diesel::update(quest_series::table.find(&series_id))
            .set(quest_series::active.eq(false))
            .execute(conn)?;
        return Ok(None);
    }

    let period_key = next_due.format("%Y-%m-%d").to_string();

    let existing = quests::table
        .filter(quests::series_id.eq(&series_id))
        .filter(quests::period_key.eq(&period_key))
        .count()
        .get_result::<i64>(conn)?;

    if existing > 0 {
        return Ok(None);
    }

    let now = utc_now();
    let max_rank = quests::table
        .select(diesel::dsl::max(quests::order_rank))
        .first::<Option<f64>>(conn)?
        .unwrap_or(0.0);

    diesel::insert_into(quests::table)
        .values((
            quests::space_id.eq(&quest.space_id),
            quests::title.eq(&quest.title),
            quests::description.eq(&quest.description.as_deref()),
            quests::status.eq("active"),
            quests::energy.eq(&quest.energy),
            quests::priority.eq(quest.priority),
            quests::due.eq(&period_key),
            quests::due_time.eq(quest.due_time.as_deref()),
            quests::repeat_rule.eq(quest.repeat_rule.as_deref()),
            quests::order_rank.eq(max_rank + 1.0),
            quests::series_id.eq(&series_id),
            quests::period_key.eq(&period_key),
            quests::created_at.eq(&now),
            quests::updated_at.eq(&now),
        ))
        .execute(conn)?;

    quests::table
        .filter(quests::series_id.eq(&series_id))
        .filter(quests::period_key.eq(&period_key))
        .select(Quest::as_select())
        .first(conn)
        .optional()
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
    notification::compute_fire_utc(due, quest.due_time.as_deref())
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
) -> Result<Option<Quest>, diesel::result::Error> {
    let all_quests: Vec<Quest> = quests::table.select(Quest::as_select()).load(conn)?;
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
        .load(conn)?;

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

pub fn resolve_active_quest(
    conn: &mut SqliteConnection,
) -> Result<Option<Quest>, diesel::result::Error> {
    resolve_active_quest_at(conn, Utc::now())
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

fn update_quest_in_db(
    conn: &mut SqliteConnection,
    id: &str,
    input: UpdateQuestInput,
) -> Result<(Quest, bool, Option<Quest>), String> {
    let now = utc_now();
    let now_dt = Utc::now();

    let existing: Quest = quests::table
        .find(id)
        .select(Quest::as_select())
        .first(conn)
        .map_err(|e| e.to_string())?;

    let mut patch = input;
    let status = patch.status.clone();
    let mut restore_should_focus = false;
    let mut period_key_to_sync: Option<String> = None;

    if let Some(rank) = patch.order_rank {
        patch.order_rank = Some(clamp_order_rank(rank));
    }

    if let (Some(series_id), Some(new_due)) = (existing.series_id.as_deref(), patch.due.as_deref())
    {
        let conflict_count = quests::table
            .filter(quests::series_id.eq(series_id))
            .filter(quests::period_key.eq(new_due))
            .filter(quests::id.ne(id))
            .count()
            .get_result::<i64>(conn)
            .map_err(|e| e.to_string())?;

        if conflict_count > 0 {
            return Err("occurrence for this date already exists in the series".to_string());
        }

        period_key_to_sync = Some(new_due.to_string());
    }

    if status.as_deref() == Some("active") {
        let restoring_from_history = existing.status != "active";
        if restoring_from_history {
            let effective_due = patch.due.as_deref().or(existing.due.as_deref());
            restore_should_focus = should_set_focus_now_for_restore(effective_due, now_dt);
        }
    }

    diesel::update(quests::table.find(id))
        .set((&patch, quests::updated_at.eq(&now)))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    if let Some(period_key) = period_key_to_sync {
        diesel::update(quests::table.find(id))
            .set(quests::period_key.eq(Some(period_key)))
            .execute(conn)
            .map_err(|e| e.to_string())?;
    }

    let completed_at_update = match status.as_deref() {
        Some("completed") => Some(Some(now.clone())),
        Some("active") | Some("abandoned") => Some(None),
        _ => None,
    };

    if let Some(val) = completed_at_update {
        diesel::update(quests::table.find(id))
            .set(quests::completed_at.eq(val))
            .execute(conn)
            .map_err(|e| e.to_string())?;
    }

    let updated_quest: Quest = quests::table
        .find(id)
        .select(Quest::as_select())
        .first(conn)
        .map_err(|e| e.to_string())?;

    // Auto-generate next occurrence when a repeating quest is completed or abandoned
    let next_occurrence = if matches!(status.as_deref(), Some("completed") | Some("abandoned")) {
        if updated_quest.repeat_rule.is_some() || updated_quest.series_id.is_some() {
            generate_next_occurrence(conn, &updated_quest).unwrap_or(None)
        } else {
            None
        }
    } else {
        None
    };

    Ok((updated_quest, restore_should_focus, next_occurrence))
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

pub fn load_quests_for_list(
    conn: &mut SqliteConnection,
) -> Result<Vec<Quest>, diesel::result::Error> {
    let loaded: Vec<Quest> = quests::table.select(Quest::as_select()).load(conn)?;
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

    Ok(loaded)
}

// ── Tauri commands ───────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_quests(state: State<DbState>) -> Result<Vec<Quest>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    load_quests_for_list(&mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_quest(
    app: tauri::AppHandle,
    state: State<DbState>,
    device_connection: State<DeviceConnectionState>,
    input: CreateQuestInput,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();

    let max_rank = quests::table
        .select(diesel::dsl::max(quests::order_rank))
        .first::<Option<f64>>(&mut *conn)
        .map_err(|e| e.to_string())?
        .unwrap_or(0.0);

    let has_repeat = input
        .repeat_rule
        .as_deref()
        .map(|r| !r.is_empty())
        .unwrap_or(false);

    if has_repeat {
        let repeat_rule_str = input.repeat_rule.clone().unwrap();
        let series_input = CreateSeriesInput {
            space_id: input.space_id.clone(),
            title: input.title.clone(),
            description: input.description.clone(),
            repeat_rule: repeat_rule_str,
            priority: input.priority,
            energy: input.energy.clone(),
        };

        let series = diesel::insert_into(quest_series::table)
            .values(&series_input)
            .returning(QuestSeries::as_returning())
            .get_result::<QuestSeries>(&mut *conn)
            .map_err(|e| e.to_string())?;

        let period_key = input
            .due
            .as_deref()
            .unwrap_or(&Utc::now().format("%Y-%m-%d").to_string())
            .to_string();

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
            .execute(&mut *conn)
            .map_err(|e| e.to_string())?;

        let created = quests::table
            .filter(quests::series_id.eq(&series.id))
            .filter(quests::period_key.eq(&period_key))
            .select(Quest::as_select())
            .first(&mut *conn)
            .map_err(|e| e.to_string())?;

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

        let payload = serde_json::to_string(&created).map_err(|e| e.to_string())?;
        emit_sync_event(
            &mut conn,
            &device_connection.identity.device_id,
            "quest",
            &created.id,
            &created.space_id,
            "upsert",
            Some(payload),
        )?;

        if created.status == "active" && created.due.is_some() {
            if let Err(e) = reminder::upsert_reminder_for_quest(&mut conn, &app, &created) {
                eprintln!(
                    "[bridge] upsert_reminder on create failed for {}: {e}",
                    created.id
                );
            }
        }

        return Ok(created);
    }

    let payload = CreateQuestInput {
        order_rank: Some(clamp_order_rank(input.order_rank.unwrap_or(max_rank + 1.0))),
        ..input
    };

    let created: Quest = diesel::insert_into(quests::table)
        .values(&payload)
        .returning(Quest::as_returning())
        .get_result(&mut *conn)
        .map_err(|e| e.to_string())?;

    let payload = serde_json::to_string(&created).map_err(|e| e.to_string())?;
    emit_sync_event(
        &mut conn,
        &device_connection.identity.device_id,
        "quest",
        &created.id,
        &created.space_id,
        "upsert",
        Some(payload),
    )?;

    if created.status == "active" && created.due.is_some() {
        if let Err(e) = reminder::upsert_reminder_for_quest(&mut conn, &app, &created) {
            eprintln!(
                "[bridge] upsert_reminder on create failed for {}: {e}",
                created.id
            );
        }
    }

    Ok(created)
}

#[tauri::command]
pub fn get_active_focus(state: State<DbState>) -> Result<Option<Quest>, String> {
    let mut conn = state.inner().0.lock().unwrap();
    resolve_active_quest(&mut conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_focus(state: State<DbState>, id: String) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let now = utc_now();

    let quest: Quest = quests::table
        .find(&id)
        .filter(quests::status.eq("active"))
        .select(Quest::as_select())
        .first(&mut *conn)
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "cannot set Focus on non-active quest".to_string())?;

    diesel::update(quests::table.find(&id))
        .set(quests::updated_at.eq(&now))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;

    append_focus_history(&mut conn, &quest.id, &quest.space_id, "manual")?;

    Ok(quest)
}

#[tauri::command]
pub fn update_quest(
    app: tauri::AppHandle,
    state: State<DbState>,
    device_connection: State<DeviceConnectionState>,
    id: String,
    input: UpdateQuestInput,
) -> Result<Quest, String> {
    let mut conn = state.inner().0.lock().unwrap();
    let previous: Quest = quests::table
        .find(&id)
        .select(Quest::as_select())
        .first(&mut *conn)
        .map_err(|e| e.to_string())?;
    let previous_status = previous.status.clone();
    let previous_space_id = previous.space_id.clone();

    let (quest, restore_should_focus, next_occurrence) = update_quest_in_db(&mut conn, &id, input)?;

    if previous_status != "active" && quest.status == "active" && restore_should_focus {
        append_focus_history(&mut conn, &quest.id, &quest.space_id, "restore")?;
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
    if let Some(ref occ) = next_occurrence {
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

#[tauri::command]
pub fn delete_quest(
    app: tauri::AppHandle,
    state: State<DbState>,
    device_connection: State<DeviceConnectionState>,
    id: String,
) -> Result<(), String> {
    let mut conn = state.inner().0.lock().unwrap();

    let space_id: String = quests::table
        .find(&id)
        .select(quests::space_id)
        .first(&mut *conn)
        .map_err(|e| e.to_string())?;

    // Cancel notifications before cascade delete removes the reminder rows
    if let Err(e) = reminder::delete_reminder_for_quest(&mut conn, &app, &id) {
        eprintln!("[bridge] delete_reminder on quest delete failed for {id}: {e}");
    }

    emit_sync_event(
        &mut conn,
        &device_connection.identity.device_id,
        "quest",
        &id,
        &space_id,
        "delete",
        None,
    )?;

    diesel::delete(quests::table.find(&id))
        .execute(&mut *conn)
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
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
}
