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
#[cfg(any(feature = "ui-plane", test))]
use crate::models::UpdateQuestCommandInput;
use crate::models::{CreateFocusHistoryInput, Quest, QuestOccurrence, OCCURRENCE_ABANDONED, OCCURRENCE_COMPLETED};
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

/// Maps the two-letter weekday tokens the UI uses ("Mo".."Su") to `chrono::Weekday`.
fn parse_weekday_token(s: &str) -> Option<chrono::Weekday> {
    use chrono::Weekday;
    match s {
        "Mo" => Some(Weekday::Mon),
        "Tu" => Some(Weekday::Tue),
        "We" => Some(Weekday::Wed),
        "Th" => Some(Weekday::Thu),
        "Fr" => Some(Weekday::Fri),
        "Sa" => Some(Weekday::Sat),
        "Su" => Some(Weekday::Sun),
        _ => None,
    }
}

/// Number of ISO weeks between the Monday of `anchor`'s week and the Monday of `date`'s week.
/// `date` is assumed to be on or after `anchor` (only ever called with forward-scanned dates).
fn weeks_since_anchor(anchor: &NaiveDate, date: &NaiveDate) -> i64 {
    use chrono::Datelike;
    let anchor_monday = *anchor - chrono::Duration::days(anchor.weekday().num_days_from_monday() as i64);
    let date_monday = *date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64);
    (date_monday - anchor_monday).num_days() / 7
}

/// Next date strictly after `current_due` whose weekday is in `days` and whose week is `interval`
/// weeks (or a multiple thereof) from `week_anchor`'s week. Bounded so a malformed/empty `days`
/// can never loop forever.
fn next_weekday_occurrence(
    current_due: &NaiveDate,
    week_anchor: &NaiveDate,
    interval: i64,
    days: &[chrono::Weekday],
) -> Option<NaiveDate> {
    use chrono::Datelike;
    let interval = interval.max(1);
    let max_scan_days = interval * 7 + 7;
    let mut candidate = *current_due + chrono::Duration::days(1);
    for _ in 0..max_scan_days {
        if days.contains(&candidate.weekday())
            && weeks_since_anchor(week_anchor, &candidate) % interval == 0
        {
            return Some(candidate);
        }
        candidate += chrono::Duration::days(1);
    }
    None
}

fn compute_next_due(
    current_due: &NaiveDate,
    rule: &RepeatRule,
    week_anchor: &NaiveDate,
) -> Option<NaiveDate> {
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
            let days: Vec<Weekday> = rule
                .days_of_week
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .filter_map(|token| parse_weekday_token(token))
                .collect();
            if !days.is_empty() {
                return next_weekday_occurrence(current_due, week_anchor, interval, &days);
            }
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

impl RepeatRule {
    fn parse(value: Option<&str>) -> Option<Self> {
        let value = value.filter(|value| !value.is_empty())?;
        let rule: RepeatRule = serde_json::from_str(value).ok()?;
        matches!(rule.preset.as_deref(), Some(preset) if preset != "none").then_some(rule)
    }

    /// The last date the rule allows (`end = "on_date"`), if any.
    fn end_date(&self) -> Option<NaiveDate> {
        (self.end.as_deref() == Some("on_date"))
            .then(|| self.end_date.as_deref())
            .flatten()
            .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
    }

    /// How many completed occurrences end the rule (`end = "after_n"`), if any.
    fn end_after(&self) -> Option<i64> {
        (self.end.as_deref() == Some("after_n")).then_some(self.end_after).flatten()
    }
}

fn parse_period_key(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

fn period_key_of(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

// ── Repeating quests (ADR-0012) ──────────────────────────────────────────────

/// The longest walk over a rule's dates: about 27 years of a daily quest.
const MAX_OCCURRENCE_STEPS: usize = 10_000;

/// A repeating quest with its stored occurrence marks. Its occurrences are
/// computed from the rule (ADR-0012 D2): a date has a row only after a
/// person closed or changed it.
pub struct RepeatingQuest<'q> {
    quest: &'q Quest,
    rule: RepeatRule,
    start: NaiveDate,
    marks: HashMap<&'q str, &'q QuestOccurrence>,
}

impl<'q> RepeatingQuest<'q> {
    /// `None` for a one-time quest (no rule, or a rule that never repeats).
    pub fn new(quest: &'q Quest, marks: &'q [QuestOccurrence]) -> Option<Self> {
        let rule = RepeatRule::parse(quest.repeat_rule.as_deref())?;
        let start = quest
            .due
            .as_deref()
            .and_then(parse_period_key)
            .or_else(|| parse_period_key(quest.created_at.get(..10)?))?;
        let marks = marks
            .iter()
            .filter(|mark| mark.quest_id == quest.id)
            .map(|mark| (mark.period_key.as_str(), mark))
            .collect();
        Some(Self { quest, rule, start, marks })
    }

    pub fn quest(&self) -> &Quest {
        self.quest
    }

    /// Every date of the rule from its start, ending at its end date.
    fn dates(&self) -> impl Iterator<Item = NaiveDate> + '_ {
        let end = self.rule.end_date();
        std::iter::successors(Some(self.start), move |date| compute_next_due(date, &self.rule, &self.start))
            .take(MAX_OCCURRENCE_STEPS)
            .take_while(move |date| end.is_none_or(|end| *date <= end))
    }

    fn mark(&self, period_key: &str) -> Option<&QuestOccurrence> {
        self.marks.get(period_key).copied()
    }

    fn is_closed(&self, period_key: &str) -> bool {
        self.mark(period_key).is_some_and(QuestOccurrence::is_closed)
    }

    fn completed_count(&self) -> i64 {
        self.marks
            .values()
            .filter(|mark| mark.status.as_deref() == Some(OCCURRENCE_COMPLETED))
            .count() as i64
    }

    /// How many more occurrences the rule allows (`after_n`), or no limit.
    fn remaining(&self) -> Option<i64> {
        self.rule.end_after().map(|max| (max - self.completed_count()).max(0))
    }

    /// The open occurrences, oldest first.
    fn open_dates(&self) -> impl Iterator<Item = String> + '_ {
        let limit = self.remaining().map_or(usize::MAX, |left| left as usize);
        self.dates()
            .map(period_key_of)
            .filter(move |key| !self.is_closed(key))
            .take(limit)
    }

    /// The occurrence that Focus and the lists show (ADR-0012 D6).
    pub fn oldest_open(&self) -> Option<String> {
        self.open_dates().next()
    }

    /// The open occurrences dated `today` or before, today's included (D7).
    pub fn open_through(&self, today: NaiveDate) -> Vec<String> {
        let today = period_key_of(today);
        self.open_dates().take_while(|key| *key <= today).collect()
    }

    /// The first open occurrence dated `day` or later: the one a reminder is
    /// for (D9).
    pub fn next_open_from(&self, day: NaiveDate) -> Option<String> {
        let day = period_key_of(day);
        self.open_dates().find(|key| *key >= day)
    }

    /// The closed occurrences, oldest first: the days in History.
    pub fn closed(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .marks
            .values()
            .filter(|mark| mark.is_closed())
            .map(|mark| mark.period_key.clone())
            .collect();
        keys.sort();
        keys
    }

    /// The quest as it is on `period_key`: the quest's fields, with the day's
    /// overrides (D4). Its `due` is the occurrence's date.
    pub fn resolve(&self, period_key: &str) -> Quest {
        let mut quest = self.quest.clone();
        quest.due = Some(period_key.to_string());
        quest.status = "active".to_string();
        quest.completed_at = None;
        if quest.is_checklist {
            quest.description = quest.description.as_deref().map(checklist::reset_unchecked);
        }
        if let Some(mark) = self.mark(period_key) {
            if let Some(title) = &mark.title {
                quest.title = title.clone();
            }
            if let Some(description) = &mark.description {
                quest.description = Some(description.clone());
                quest.checklist_base = mark.checklist_base.clone();
            }
            if let Some(energy) = mark.energy {
                quest.energy = energy;
            }
            if let Some(priority) = mark.priority {
                quest.priority = priority;
            }
            if let Some(due_time) = &mark.due_time {
                quest.due_time = Some(due_time.clone());
            }
            if let Some(status) = &mark.status {
                quest.status = status.clone();
                quest.completed_at = (status == OCCURRENCE_COMPLETED).then(|| mark.closed_at.clone()).flatten();
                quest.updated_at = mark.updated_at.clone();
            }
        }
        quest
    }

    /// The day's stored mark, or a new empty one.
    fn mark_or_new(&self, period_key: &str) -> QuestOccurrence {
        self.mark(period_key)
            .cloned()
            .unwrap_or_else(|| QuestOccurrence::new(&self.quest.id, period_key, &utc_now()))
    }
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

    fallback.first().map(|quest| (*quest).clone())
}

fn resolve_active_quest_at(
    conn: &mut SqliteConnection,
    now: DateTime<Utc>,
) -> Result<Option<Quest>, String> {
    let active_quests = QuestService::new(conn).active_quests()?;
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
                            listed.push(repeating.resolve(&period_key));
                        }
                    }
                    for period_key in repeating.closed() {
                        listed.push(repeating.resolve(&period_key));
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
                .map(|period_key| repeating.resolve(&period_key))
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
            .map(|period_key| repeating.resolve(&period_key)))
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
        let day = repeating.resolve(period_key);
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
    /// oldest open date; with marks the rule gets an end at the last closed
    /// day, and the quest ends.
    fn stop_repeat(&mut self, repeating: &RepeatingQuest<'_>, origin_device_id: Option<&str>) -> Result<QuestChange, String> {
        let quest = repeating.quest().clone();
        let closed = repeating.closed();
        let input = match closed.last() {
            None => UpdateQuestInputBuilder::default()
                .due(repeating.oldest_open())
                .build(),
            Some(last_closed) => {
                let mut rule = repeating.rule.clone();
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
                    (Some(open), _) => repeating.resolve(&open),
                    (None, Some(day)) => repeating.resolve(day),
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
                let day = repeating.resolve(&period_key);
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
    due: Option<String>,
    repeat_rule: Option<String>,
}

impl UpdateQuestInputBuilder {
    fn status(mut self, status: &str) -> Self {
        self.status = Some(status.to_string());
        self
    }

    fn due(mut self, due: Option<String>) -> Self {
        self.due = due;
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
            due: self.due,
            due_time: None,
            repeat_rule: self.repeat_rule,
            order_rank: None,
            is_checklist: None,
        }
    }
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

    fn complete_and_generate_next(conn: &mut SqliteConnection, quest_id: &str) -> Option<Quest> {
        diesel::update(quests::table.find(quest_id))
            .set(quests::status.eq("completed"))
            .execute(conn)
            .expect("complete occurrence");
        let completed: Quest = quests::table
            .find(quest_id)
            .select(Quest::as_select())
            .first(conn)
            .expect("reload completed quest");
        generate_next_occurrence(conn, &completed).expect("generate next occurrence")
    }

    #[test]
    fn recurring_custom_interval_without_weekdays_uses_unit_step() {
        let db_path = temp_db_path("recurring-custom-unit-step");
        let mut conn = open_db_at_path(&db_path);

        let repeat_rule = r#"{"preset":"custom","interval":2,"unit":"week"}"#;
        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("Biweekly custom"),
                quests::status.eq("active"),
                quests::due.eq("2026-04-06"),
                quests::repeat_rule.eq(repeat_rule),
                quests::created_at.eq("2026-04-06T10:00:00Z"),
                quests::updated_at.eq("2026-04-06T10:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert custom-interval quest");

        let first: Quest = quests::table
            .filter(quests::title.eq("Biweekly custom"))
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("load first occurrence");

        let next = complete_and_generate_next(&mut conn, &first.id)
            .expect("next occurrence must be generated");
        assert_eq!(
            next.due.as_deref(),
            Some("2026-04-20"),
            "empty days_of_week must fall back to plain interval*unit stepping"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn recurring_custom_weekly_by_weekday_honors_interval_and_anchor() {
        let db_path = temp_db_path("recurring-custom-weekday-interval");
        let mut conn = open_db_at_path(&db_path);

        // 2026-04-06 is a Monday; anchor week == this quest's own creation week.
        let repeat_rule = r#"{"preset":"custom","interval":2,"unit":"day","days_of_week":["Mo","We"]}"#;
        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("1"),
                quests::title.eq("Every 2 weeks Mon/Wed"),
                quests::status.eq("active"),
                quests::due.eq("2026-04-06"),
                quests::repeat_rule.eq(repeat_rule),
                quests::created_at.eq("2026-04-06T10:00:00Z"),
                quests::updated_at.eq("2026-04-06T10:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert weekday-interval quest");

        let first: Quest = quests::table
            .filter(quests::title.eq("Every 2 weeks Mon/Wed"))
            .select(Quest::as_select())
            .first(&mut conn)
            .expect("load first occurrence");

        let second = complete_and_generate_next(&mut conn, &first.id)
            .expect("second occurrence must be generated");
        assert_eq!(
            second.due.as_deref(),
            Some("2026-04-08"),
            "Wednesday of the same (anchor) week is still an active week"
        );

        let third = complete_and_generate_next(&mut conn, &second.id)
            .expect("third occurrence must be generated");
        assert_eq!(
            third.due.as_deref(),
            Some("2026-04-20"),
            "the intervening week is skipped; next active week's Monday is 2 weeks after the anchor"
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
                priority: crate::models::PRIORITY_MEDIUM,
                energy: crate::models::ENERGY_MEDIUM,
                is_checklist: false,
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
            .expect("create checklist quest")
            .quest;

        log_checklist_activity(
            &mut conn,
            &quest.id,
            "added",
            "Added \"headphones\"",
            Some("device-local"),
        )
        .expect("log checklist activity");

        emit_quest_sync_events(&mut conn, "device-local", &quest.space_id, &quest)
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
            .expect("create checklist quest")
            .quest;

        log_checklist_activity(
            &mut conn,
            &quest.id,
            "added",
            "Added \"headphones\"",
            Some("device-local"),
        )
        .expect("log checklist activity");

        emit_quest_sync_events(&mut conn, "device-local", &quest.space_id, &quest)
            .expect("emit initial quest sync with checklist activity");

        let toggled = QuestService::new(&mut conn)
            .toggle_checklist_item(&quest.id, "a1", true)
            .expect("toggle checklist item");
        emit_quest_sync_events(&mut conn, "device-local", &toggled.space_id, &toggled)
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
            .expect("create checklist quest")
            .quest;

        log_checklist_activity(
            &mut conn,
            &quest.id,
            "added",
            "Added \"headphones\"",
            Some("device-local"),
        )
        .expect("log checklist activity");

        emit_quest_sync_events(&mut conn, "device-local", &quest.space_id, &quest)
            .expect("emit initial quest sync with checklist activity");

        let (moved, _, _) = update_quest_in_db(&mut conn, &quest.id, space_patch("2"))
            .expect("move checklist to space 2");
        emit_quest_sync_events(&mut conn, "device-local", &quest.space_id, &moved)
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

    #[test]
    fn future_series_checklist_sync_uses_current_occurrence_space() {
        let db_path = temp_db_path("future-series-checklist-sync-occurrence-space");
        let mut conn = open_db_at_path(&db_path);

        let repeat_rule = r#"{"preset":"weekly"}"#;
        let series = diesel::insert_into(quest_series::table)
            .values(&CreateSeriesInput {
                space_id: "1".to_string(),
                title: "Moved checklist".to_string(),
                description: Some("- [ ] old <!--k=a1-->".to_string()),
                repeat_rule: repeat_rule.to_string(),
                priority: crate::models::PRIORITY_MEDIUM,
                energy: crate::models::ENERGY_MEDIUM,
                is_checklist: true,
            })
            .returning(QuestSeries::as_returning())
            .get_result::<QuestSeries>(&mut conn)
            .expect("insert series");

        diesel::insert_into(quests::table)
            .values((
                quests::space_id.eq("2"),
                quests::title.eq("Moved checklist"),
                quests::description.eq(Some("- [ ] old <!--k=a1-->")),
                quests::status.eq("active"),
                quests::due.eq("2026-05-15"),
                quests::repeat_rule.eq(repeat_rule),
                quests::series_id.eq(&series.id),
                quests::period_key.eq("2026-05-15"),
                quests::is_checklist.eq(true),
                quests::created_at.eq("2026-05-15T10:00:00Z"),
                quests::updated_at.eq("2026-05-15T10:00:00Z"),
            ))
            .execute(&mut conn)
            .expect("insert moved occurrence");

        let occurrence_id: String = quests::table
            .filter(quests::series_id.eq(&series.id))
            .select(quests::id)
            .first(&mut conn)
            .expect("load occurrence id");

        let updated = QuestService::new(&mut conn)
            .update_series_checklist(
                &series.id,
                &occurrence_id,
                "- [ ] new <!--k=a1-->",
                "future",
                Some("device-local"),
            )
            .expect("update future checklist");

        assert_eq!(updated.space_id, "2", "occurrence stays in its moved space");
        assert_eq!(
            QuestRepository::new(&mut conn)
                .series_space_id(&series.id)
                .expect("load series space"),
            "1",
            "series row can still retain the original space"
        );

        emit_quest_checklist_sync(&mut conn, "device-local", &updated)
            .expect("emit occurrence checklist sync");
        emit_series_checklist_template_sync(
            &mut conn,
            "device-local",
            &series.id,
            &updated.space_id,
        )
        .expect("emit series template sync");

        let rows: Vec<(String, String, String, Option<String>)> = sync_outbox::table
            .order(sync_outbox::created_at.asc())
            .select((
                sync_outbox::entity_type,
                sync_outbox::op_type,
                sync_outbox::space_id,
                sync_outbox::payload,
            ))
            .load(&mut conn)
            .expect("load sync outbox rows");

        let series_row = rows
            .iter()
            .find(|(entity_type, op_type, _, _)| {
                entity_type == "quest_series" && op_type == "upsert"
            })
            .expect("series template sync row");
        assert_eq!(
            series_row.2, "2",
            "series template update must route through the moved occurrence space"
        );
        let payload = series_row.3.as_ref().expect("series payload");
        let payload: QuestSeries =
            serde_json::from_str(payload).expect("deserialize series payload");
        assert_eq!(
            payload.space_id, "1",
            "outbox routing space must not require mutating the series row"
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
                priority: crate::models::PRIORITY_MEDIUM,
                energy: crate::models::ENERGY_MEDIUM,
                is_checklist: false,
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
                priority: crate::models::PRIORITY_MEDIUM,
                energy: crate::models::ENERGY_MEDIUM,
                is_checklist: false,
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
                priority: crate::models::PRIORITY_MEDIUM,
                energy: crate::models::ENERGY_MEDIUM,
                is_checklist: false,
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
        result.quest
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
        result.quest
    }

    #[test]
    fn checklist_item_crud_logs_activity_but_toggles_do_not() {
        let db_path = temp_db_path("checklist-crud");
        let mut conn = open_db_at_path(&db_path);

        let quest = create_active_quest_with_checklist(&mut conn, "");
        let mut service = QuestService::new(&mut conn);

        let quest = service
            .add_checklist_item(&quest.id, "headphones", Some("dev-a"))
            .expect("add item");
        let item_id = checklist::parse(quest.description.as_deref().unwrap())[0]
            .id
            .clone();

        let quest = service
            .toggle_checklist_item(&quest.id, &item_id, true)
            .expect("toggle item");
        assert!(checklist::parse(quest.description.as_deref().unwrap())[0].checked);

        let quest = service
            .edit_checklist_item_text(&quest.id, &item_id, "over-ear headphones", Some("dev-a"))
            .expect("edit item text");
        assert_eq!(
            checklist::parse(quest.description.as_deref().unwrap())[0].text,
            "over-ear headphones"
        );

        let quest = service
            .remove_checklist_item(&quest.id, &item_id, Some("dev-a"))
            .expect("remove item");
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

        let result = QuestService::new(&mut conn).toggle_checklist_item(&quest.id, "missing", true);
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
            QuestService::new(&mut conn).remove_checklist_item(&quest.id, "missing", Some("dev-a"));
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

            let result = service.toggle_checklist_item(&quest.id, "a1", true);
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
            service.add_checklist_item(&quest.id, "headphones", Some("dev-a")),
            service.toggle_checklist_item(&quest.id, "a1", true),
            service.edit_checklist_item_text(&quest.id, "a1", "headphones", Some("dev-a")),
            service.remove_checklist_item(&quest.id, "a1", Some("dev-a")),
            service.reorder_checklist(&quest.id, &["a1".to_string()]),
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
    fn update_series_checklist_rejects_prose_and_mismatched_occurrences() {
        let db_path = temp_db_path("checklist-series-reject-invalid");
        let mut conn = open_db_at_path(&db_path);

        let prose_result = QuestService::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Recurring notes".to_string(),
                description: Some("meeting notes".to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: Some("2026-04-01".to_string()),
                due_time: None,
                repeat_rule: Some(r#"{"preset":"daily"}"#.to_string()),
                order_rank: None,
                is_checklist: false,
            })
            .expect("create repeating prose quest");
        let prose_quest = prose_result.quest;
        let prose_series = prose_result
            .series
            .expect("repeating prose quest must create a series");

        for scope in ["this", "future"] {
            let result = QuestService::new(&mut conn).update_series_checklist(
                &prose_series.id,
                &prose_quest.id,
                "- [ ] checklist item <!--k=a1-->",
                scope,
                None,
            );
            assert!(matches!(
                result,
                Err(ref error) if error == "quest is not a checklist"
            ));
        }

        let stored_prose = QuestService::new(&mut conn).get(&prose_quest.id).unwrap();
        assert!(
            !stored_prose.is_checklist,
            "set-template must not convert prose quests"
        );
        assert_eq!(stored_prose.description.as_deref(), Some("meeting notes"));

        let stored_prose_series: QuestSeries = crate::schema::quest_series::table
            .find(&prose_series.id)
            .select(QuestSeries::as_select())
            .first(&mut conn)
            .unwrap();
        assert!(
            !stored_prose_series.is_checklist,
            "rejected future scope must not convert series"
        );
        assert_eq!(
            stored_prose_series.description.as_deref(),
            Some("meeting notes")
        );

        let first_checklist = QuestService::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Checklist one".to_string(),
                description: Some("- [ ] one <!--k=a1-->".to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: Some("2026-04-02".to_string()),
                due_time: None,
                repeat_rule: Some(r#"{"preset":"daily"}"#.to_string()),
                order_rank: None,
                is_checklist: true,
            })
            .expect("create first checklist series");
        let first_series = first_checklist.series.expect("first checklist series");
        let second_checklist = QuestService::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Checklist two".to_string(),
                description: Some("- [ ] two <!--k=b1-->".to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: Some("2026-04-03".to_string()),
                due_time: None,
                repeat_rule: Some(r#"{"preset":"daily"}"#.to_string()),
                order_rank: None,
                is_checklist: true,
            })
            .expect("create second checklist series");
        let second_quest = second_checklist.quest;

        let result = QuestService::new(&mut conn).update_series_checklist(
            &first_series.id,
            &second_quest.id,
            "- [ ] wrong series <!--k=x1-->",
            "this",
            None,
        );
        assert!(matches!(
            result,
            Err(ref error) if error == "quest is not an occurrence in this series"
        ));
        let stored_second = QuestService::new(&mut conn).get(&second_quest.id).unwrap();
        assert_eq!(
            stored_second.description.as_deref(),
            Some("- [ ] two <!--k=b1-->")
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
            .add_checklist_item(&quest.id, "late addition", Some("dev-a"))
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

    #[test]
    fn get_series_checklist_template_reads_the_stored_template_not_the_occurrence() {
        let db_path = temp_db_path("checklist-get-series-template");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[checklist::ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: false,
        }]);
        let result = QuestService::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Go to office".to_string(),
                description: Some(template.clone()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: Some("2026-04-01".to_string()),
                due_time: None,
                repeat_rule: Some(r#"{"preset":"daily"}"#.to_string()),
                order_rank: None,
                is_checklist: true,
            })
            .expect("create repeating quest with checklist");
        let quest = result.quest;
        let series = result.series.expect("repeating quest must create a series");

        // "This occurrence" only affects the occurrence — the template read must be unaffected.
        let occurrence_only_checklist = checklist::add_item(Some(&template), "today only");
        QuestService::new(&mut conn)
            .update_series_checklist(
                &series.id,
                &quest.id,
                &occurrence_only_checklist,
                "this",
                None,
            )
            .expect("this-occurrence scoped update");

        let stored_template = QuestService::new(&mut conn)
            .get_series_checklist_template(&series.id)
            .expect("load series template");
        assert_eq!(
            stored_template.as_deref(),
            Some(template.as_str()),
            "get_series_checklist_template must return the series' own stored template, not \
             whatever the current occurrence happens to look like"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn update_series_checklist_this_scope_leaves_template_untouched() {
        let db_path = temp_db_path("checklist-scope-this");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[checklist::ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: false,
        }]);
        let result = QuestService::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Go to office".to_string(),
                description: Some(template.clone()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: Some("2026-04-01".to_string()),
                due_time: None,
                repeat_rule: Some(r#"{"preset":"daily"}"#.to_string()),
                order_rank: None,
                is_checklist: true,
            })
            .expect("create repeating quest with checklist");
        let quest = result.quest;
        let series = result.series.expect("repeating quest must create a series");

        let occurrence_only_checklist =
            checklist::add_item(Some(&template), "extra for today only");
        QuestService::new(&mut conn)
            .update_series_checklist(
                &series.id,
                &quest.id,
                &occurrence_only_checklist,
                "this",
                None,
            )
            .expect("this-occurrence scoped update");

        let stored_series: QuestSeries = crate::schema::quest_series::table
            .find(&series.id)
            .select(QuestSeries::as_select())
            .first(&mut conn)
            .unwrap();
        assert_eq!(
            stored_series.description.as_deref(),
            Some(template.as_str()),
            "'this occurrence' scope must not touch the series template"
        );

        let stored_quest = QuestService::new(&mut conn).get(&quest.id).unwrap();
        assert_eq!(
            checklist::parse(stored_quest.description.as_deref().unwrap()).len(),
            2,
            "the occurrence itself must reflect the this-scoped edit"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn update_series_checklist_this_scope_logs_replacement_activity() {
        let db_path = temp_db_path("checklist-scope-this-activity");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[checklist::ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: false,
        }]);
        let result = QuestService::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Go to office".to_string(),
                description: Some(template.clone()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: Some("2026-04-01".to_string()),
                due_time: None,
                repeat_rule: Some(r#"{"preset":"daily"}"#.to_string()),
                order_rank: None,
                is_checklist: true,
            })
            .expect("create repeating quest with checklist");
        let quest = result.quest;
        let series = result.series.expect("repeating quest must create a series");

        // "This occurrence" set-template must be audited the same as add/remove/edit — issue
        // #128's history requirement doesn't stop applying just because the whole checklist was
        // replaced in one call instead of one item at a time.
        let replacement = checklist::add_item(Some(&template), "lunch");
        QuestService::new(&mut conn)
            .update_series_checklist(&series.id, &quest.id, &replacement, "this", Some("dev-a"))
            .expect("this-scope replacement");

        let activity = QuestService::new(&mut conn)
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        assert_eq!(
            activity.iter().map(|a| a.kind.as_str()).collect::<Vec<_>>(),
            vec!["added"],
            "this-scope set-template must log the structural diff, not silently replace"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn update_series_checklist_rejects_non_active_occurrence_for_both_scopes() {
        for scope in ["this", "future"] {
            let db_path = temp_db_path(&format!("checklist-scope-non-active-{scope}"));
            let mut conn = open_db_at_path(&db_path);

            let template = checklist::serialize(&[checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            }]);
            let result = QuestService::new(&mut conn)
                .create(CreateQuestInput {
                    space_id: "1".to_string(),
                    title: "Go to office".to_string(),
                    description: Some(template.clone()),
                    energy: crate::models::ENERGY_MEDIUM,
                    priority: crate::models::PRIORITY_MEDIUM,
                    due: Some("2026-04-01".to_string()),
                    due_time: None,
                    repeat_rule: Some(r#"{"preset":"daily"}"#.to_string()),
                    order_rank: None,
                    is_checklist: true,
                })
                .expect("create repeating quest with checklist");
            let quest = result.quest;
            let series = result.series.expect("repeating quest must create a series");

            QuestService::new(&mut conn)
                .update(&quest.id, status_patch("completed"))
                .expect("complete occurrence");

            // A completed occurrence's checklist is historical — set-template must not be able
            // to silently rewrite it outside the audited post-completion-edit path.
            let result = QuestService::new(&mut conn).update_series_checklist(
                &series.id,
                &quest.id,
                &checklist::add_item(Some(&template), "lunch"),
                scope,
                Some("dev-a"),
            );
            assert!(
                result.is_err(),
                "set-template on a completed occurrence must be rejected for scope={scope}"
            );

            let _ = std::fs::remove_file(db_path);
        }
    }

    #[test]
    fn update_series_checklist_future_scope_updates_template_and_reconciles_occurrence() {
        let db_path = temp_db_path("checklist-scope-future");
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
                checked: false,
            },
        ]);
        let result = QuestService::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Go to office".to_string(),
                description: Some(template),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: Some("2026-04-01".to_string()),
                due_time: None,
                repeat_rule: Some(r#"{"preset":"daily"}"#.to_string()),
                order_rank: None,
                is_checklist: true,
            })
            .expect("create repeating quest with checklist");
        let quest = result.quest;
        let series = result.series.expect("repeating quest must create a series");

        // User checks headphones on today's occurrence before editing the template.
        QuestService::new(&mut conn)
            .toggle_checklist_item(&quest.id, "a1", true)
            .expect("toggle item");

        // "This and future occurrences": drop the key fob, add a badge, keep headphones.
        let new_template = checklist::serialize(&[
            checklist::ChecklistItem {
                id: "a1".into(),
                text: "headphones".into(),
                checked: false,
            },
            checklist::ChecklistItem {
                id: "a3".into(),
                text: "badge".into(),
                checked: false,
            },
        ]);
        QuestService::new(&mut conn)
            .update_series_checklist(&series.id, &quest.id, &new_template, "future", None)
            .expect("future-scoped update");

        let stored_series: QuestSeries = crate::schema::quest_series::table
            .find(&series.id)
            .select(QuestSeries::as_select())
            .first(&mut conn)
            .unwrap();
        assert_eq!(
            stored_series.description.as_deref(),
            Some(new_template.as_str()),
            "'this and future' scope must update the series template"
        );

        let stored_quest = QuestService::new(&mut conn).get(&quest.id).unwrap();
        let items = checklist::parse(stored_quest.description.as_deref().unwrap());
        assert!(
            items.iter().find(|it| it.id == "a1").unwrap().checked,
            "unchanged current-occurrence item must keep its check"
        );
        assert!(
            !items.iter().find(|it| it.id == "a3").unwrap().checked,
            "newly added template item must be unchecked"
        );
        assert!(
            items.iter().find(|it| it.id == "a2").is_none(),
            "template-removed item must be dropped from the active occurrence"
        );

        let activity = QuestService::new(&mut conn)
            .get_checklist_activity(&quest.id)
            .expect("load activity");
        assert!(
            activity
                .iter()
                .any(|row| row.kind == "added" && row.detail == "Added \"badge\""),
            "future-scope reconciliation must audit the item added to the current occurrence"
        );
        assert!(
            activity
                .iter()
                .any(|row| row.kind == "removed" && row.detail == "Removed \"key fob\""),
            "future-scope reconciliation must audit the item removed from the current occurrence"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn recurring_occurrence_retains_explicit_large_energy_and_high_priority() {
        let db_path = temp_db_path("recurrence-retains-energy-priority");
        let mut conn = open_db_at_path(&db_path);
        let result = QuestService::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Prepare launch".to_string(),
                description: None,
                energy: crate::models::ENERGY_LARGE,
                priority: crate::models::PRIORITY_HIGH,
                due: Some("2026-04-01".to_string()),
                due_time: None,
                repeat_rule: Some(r#"{"preset":"daily"}"#.to_string()),
                order_rank: None,
                is_checklist: false,
            })
            .expect("create recurring quest with explicit metadata");
        let quest = result.quest;

        let next = QuestService::new(&mut conn)
            .update(&quest.id, status_patch("completed"))
            .expect("complete recurring quest")
            .next_occurrence
            .expect("completion must generate the next occurrence");

        assert_eq!(next.energy, crate::models::ENERGY_LARGE);
        assert_eq!(next.priority, crate::models::PRIORITY_HIGH);

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn recurring_occurrence_copies_checklist_template_fresh_and_unchecked() {
        let db_path = temp_db_path("checklist-recurrence-copy");
        let mut conn = open_db_at_path(&db_path);

        let template = checklist::serialize(&[checklist::ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: false,
        }]);
        let result = QuestService::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Go to office".to_string(),
                description: Some(template),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: Some("2026-04-01".to_string()),
                due_time: None,
                repeat_rule: Some(r#"{"preset":"daily"}"#.to_string()),
                order_rank: None,
                is_checklist: true,
            })
            .expect("create repeating quest with checklist");
        let quest = result.quest;

        // Check the item, then complete the occurrence — completion must generate the next
        // occurrence with a fresh, unchecked copy of the series template (#128).
        QuestService::new(&mut conn)
            .toggle_checklist_item(&quest.id, "a1", true)
            .expect("toggle item");

        let update_result = QuestService::new(&mut conn)
            .update(&quest.id, status_patch("completed"))
            .expect("complete occurrence");
        let next = update_result
            .next_occurrence
            .expect("completing a recurring quest must generate the next occurrence");

        let next_items = checklist::parse(next.description.as_deref().unwrap());
        assert_eq!(next_items.len(), 1);
        assert!(
            !next_items[0].checked,
            "next occurrence's checklist must start fully unchecked"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn recurring_non_checklist_occurrence_copies_current_prose_description() {
        let db_path = temp_db_path("non-checklist-recurrence-current-description");
        let mut conn = open_db_at_path(&db_path);

        let result = QuestService::new(&mut conn)
            .create(CreateQuestInput {
                space_id: "1".to_string(),
                title: "Write journal".to_string(),
                description: Some("Original notes".to_string()),
                energy: crate::models::ENERGY_MEDIUM,
                priority: crate::models::PRIORITY_MEDIUM,
                due: Some("2026-04-01".to_string()),
                due_time: None,
                repeat_rule: Some(r#"{"preset":"daily"}"#.to_string()),
                order_rank: None,
                is_checklist: false,
            })
            .expect("create repeating prose quest");
        let quest = result.quest;
        let series = result.series.expect("repeating quest must create a series");

        QuestService::new(&mut conn)
            .update_patch(
                &quest.id,
                QuestUpdatePatch {
                    input: UpdateQuestInput {
                        description: Some("Edited occurrence notes".to_string()),
                        ..status_patch("active")
                    },
                    description: QuestFieldPatch::Set("Edited occurrence notes".to_string()),
                    due: QuestFieldPatch::Unchanged,
                    due_time: QuestFieldPatch::Unchanged,
                    repeat_rule: QuestFieldPatch::Unchanged,
                },
            )
            .expect("edit current occurrence prose");

        let stored_series: QuestSeries = crate::schema::quest_series::table
            .find(&series.id)
            .select(QuestSeries::as_select())
            .first(&mut conn)
            .unwrap();
        assert_eq!(
            stored_series.description.as_deref(),
            Some("Original notes"),
            "normal prose edits must not rewrite the series row"
        );

        let update_result = QuestService::new(&mut conn)
            .update(&quest.id, status_patch("completed"))
            .expect("complete occurrence");
        let next = update_result
            .next_occurrence
            .expect("completing a recurring quest must generate the next occurrence");

        assert_eq!(
            next.description.as_deref(),
            Some("Edited occurrence notes"),
            "non-checklist recurrence should carry current prose notes forward"
        );
        assert!(!next.is_checklist);
        assert!(next.checklist_base.is_none());

        let _ = std::fs::remove_file(db_path);
    }
}
