use chrono::NaiveDate;
use diesel::prelude::*;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;

use crate::models::{QuestOccurrence, OCCURRENCE_COMPLETED};
use crate::schema::quests;
use crate::services::checklist;
use crate::services::db::utc_now;

pub const ENERGY_SMALL: i64 = 1;
pub const ENERGY_MEDIUM: i64 = 2;
pub const ENERGY_LARGE: i64 = 3;
pub const PRIORITY_LOW: i64 = 1;
pub const PRIORITY_MEDIUM: i64 = 2;
pub const PRIORITY_HIGH: i64 = 3;

pub fn parse_energy_name(value: &str) -> Option<i64> {
    match value {
        "small" => Some(ENERGY_SMALL),
        "medium" => Some(ENERGY_MEDIUM),
        "large" => Some(ENERGY_LARGE),
        _ => None,
    }
}

pub fn parse_priority_name(value: &str) -> Option<i64> {
    match value {
        "low" => Some(PRIORITY_LOW),
        "medium" => Some(PRIORITY_MEDIUM),
        "high" => Some(PRIORITY_HIGH),
        _ => None,
    }
}

pub fn energy_name(value: i64) -> &'static str {
    match value {
        ENERGY_SMALL => "small",
        ENERGY_LARGE => "large",
        _ => "medium",
    }
}

pub fn priority_name(value: i64) -> &'static str {
    match value {
        PRIORITY_LOW => "low",
        PRIORITY_HIGH => "high",
        _ => "medium",
    }
}

pub(crate) fn serialize_energy<S>(value: &i64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(energy_name(*value))
}

pub(crate) fn serialize_priority<S>(value: &i64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(priority_name(*value))
}

pub(crate) fn deserialize_energy<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    parse_energy_name(&value).ok_or_else(|| {
        serde::de::Error::custom(format!(
            "invalid energy {value:?}; expected small, medium, or large"
        ))
    })
}

pub(crate) fn deserialize_priority<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    parse_priority_name(&value).ok_or_else(|| {
        serde::de::Error::custom(format!(
            "invalid priority {value:?}; expected low, medium, or high"
        ))
    })
}

fn deserialize_optional_energy<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|value| {
            parse_energy_name(&value).ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "invalid energy {value:?}; expected small, medium, or large"
                ))
            })
        })
        .transpose()
}

fn deserialize_optional_priority<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|value| {
            parse_priority_name(&value).ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "invalid priority {value:?}; expected low, medium, or high"
                ))
            })
        })
        .transpose()
}

#[derive(Queryable, Selectable, Serialize, Deserialize, Clone)]
#[diesel(table_name = quests)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Quest {
    pub id: String,
    pub space_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    /// Stored as 1 = small, 2 = medium, 3 = large. Serialized as names.
    #[serde(
        serialize_with = "serialize_energy",
        deserialize_with = "deserialize_energy"
    )]
    pub energy: i64,
    /// Stored as 1 = low, 2 = medium, 3 = high. Serialized as names.
    #[serde(
        serialize_with = "serialize_priority",
        deserialize_with = "deserialize_priority"
    )]
    pub priority: i64,
    pub due: Option<String>,
    pub due_time: Option<String>,
    /// JSON-encoded RepeatRule, or null
    pub repeat_rule: Option<String>,
    pub completed_at: Option<String>,
    pub order_rank: f64,
    #[serde(default)]
    pub focus_enter_count: i64,
    pub created_at: String,
    pub updated_at: String,
    /// When true, `description` is authored/rendered as a checklist (task-list text,
    /// `- [ ] text <!--k=id-->` lines) instead of prose — issue #128. There is no separate
    /// checklist content column: the description field itself holds that task-list text, and a
    /// checklist quest simply parses/renders that same field as a task list.
    #[serde(default)]
    pub is_checklist: bool,
    /// Device-local convergence bookkeeping for the per-item checklist merge when
    /// `is_checklist` — the last `description` value both sides last agreed on. Never included
    /// in sync payloads.
    #[serde(skip_serializing, default)]
    pub checklist_base: Option<String>,
}

#[derive(Deserialize, Insertable)]
#[diesel(table_name = quests)]
pub struct CreateQuestInput {
    #[serde(default = "default_space_id")]
    pub space_id: String,
    pub title: String,
    pub description: Option<String>,
    #[serde(default = "default_energy", deserialize_with = "deserialize_energy")]
    pub energy: i64,
    #[serde(
        default = "default_priority",
        deserialize_with = "deserialize_priority"
    )]
    pub priority: i64,
    pub due: Option<String>,
    pub due_time: Option<String>,
    pub repeat_rule: Option<String>,
    pub order_rank: Option<f64>,
    /// Marks this quest as a checklist quest — `description` is authored as task-list text.
    #[serde(default)]
    pub is_checklist: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestFieldPatch<T> {
    Unchanged,
    Set(T),
    Clear,
}

impl<T> Default for QuestFieldPatch<T> {
    fn default() -> Self {
        Self::Unchanged
    }
}

/// A missing JSON key deserializes via `#[serde(default)]` to `Unchanged` (this impl is never
/// called); a present `null` deserializes here to `Clear`; a present value deserializes to `Set`.
impl<'de, T: Deserialize<'de>> Deserialize<'de> for QuestFieldPatch<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => QuestFieldPatch::Set(value),
            None => QuestFieldPatch::Clear,
        })
    }
}

/// Explicit nullable update contract. Transport adapters map omitted values,
/// ordinary text (including empty text), and literal null to these variants.
pub struct QuestUpdatePatch {
    pub input: UpdateQuestInput,
    pub description: QuestFieldPatch<String>,
    pub due: QuestFieldPatch<String>,
    pub due_time: QuestFieldPatch<String>,
    pub repeat_rule: QuestFieldPatch<String>,
}

impl Quest {
    /// The moment the quest comes: `due` + `due_time`, local time (09:00 when
    /// there is no time). For a repeating quest, of its shown day.
    pub fn due_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        crate::services::due_time::compute_fire_utc(self.due.as_deref()?, self.due_time.as_deref())
    }
}

impl QuestUpdatePatch {
    /// The fields of `input`; the clearable fields stay as they are.
    pub fn unchanged(input: UpdateQuestInput) -> Self {
        Self {
            input,
            description: QuestFieldPatch::Unchanged,
            due: QuestFieldPatch::Unchanged,
            due_time: QuestFieldPatch::Unchanged,
            repeat_rule: QuestFieldPatch::Unchanged,
        }
    }
}

#[derive(Deserialize, AsChangeset)]
#[diesel(table_name = quests)]
pub struct UpdateQuestInput {
    pub space_id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_energy")]
    pub energy: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_priority")]
    pub priority: Option<i64>,
    pub due: Option<String>,
    pub due_time: Option<String>,
    pub repeat_rule: Option<String>,
    pub order_rank: Option<f64>,
    pub is_checklist: Option<bool>,
}

/// Wire-level input for the `update_quest` Tauri command. Unlike `UpdateQuestInput`
/// (a plain `AsChangeset` where JSON `null` and an omitted key are indistinguishable —
/// both become `None`, meaning "leave unchanged"), `description`/`due`/`due_time`/
/// `repeat_rule` here use `QuestFieldPatch` so an explicit JSON `null` is distinguishable
/// from an omitted key and actually clears the column. Mirrors the CLI's existing
/// `QuestUpdatePatch` contract (see `services::cli::update_quest_from_cli`).
#[derive(Deserialize)]
pub struct UpdateQuestCommandInput {
    pub space_id: Option<String>,
    pub title: Option<String>,
    #[serde(default)]
    pub description: QuestFieldPatch<String>,
    pub status: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_energy")]
    pub energy: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_priority")]
    pub priority: Option<i64>,
    #[serde(default)]
    pub due: QuestFieldPatch<String>,
    #[serde(default)]
    pub due_time: QuestFieldPatch<String>,
    #[serde(default)]
    pub repeat_rule: QuestFieldPatch<String>,
    pub order_rank: Option<f64>,
    pub is_checklist: Option<bool>,
    /// A repeating quest only (ADR-0012 D8): `"day"` writes the edit on one
    /// occurrence, `"all"` (the default) on the quest.
    #[serde(default)]
    pub scope: Option<String>,
    /// A repeating quest only: the occurrence the edit is for. Missing means
    /// its oldest open occurrence.
    #[serde(default)]
    pub period_key: Option<String>,
}

impl UpdateQuestCommandInput {
    pub fn into_patch(self) -> QuestUpdatePatch {
        QuestUpdatePatch {
            input: UpdateQuestInput {
                space_id: self.space_id,
                title: self.title,
                description: None,
                status: self.status,
                energy: self.energy,
                priority: self.priority,
                due: None,
                due_time: None,
                repeat_rule: None,
                order_rank: self.order_rank,
                is_checklist: self.is_checklist,
            },
            description: self.description,
            due: self.due,
            due_time: self.due_time,
            repeat_rule: self.repeat_rule,
        }
    }
}

pub fn default_priority() -> i64 {
    PRIORITY_MEDIUM
}

pub fn default_space_id() -> String {
    "1".to_string()
}

pub fn default_energy() -> i64 {
    ENERGY_MEDIUM
}

pub fn clamp_order_rank(value: f64) -> f64 {
    value.clamp(-100.0, 100.0)
}

// ── Repeat rule ──────────────────────────────────────────────────────────────

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct RepeatRule {
    pub preset: Option<String>,
    pub interval: Option<i64>,
    pub unit: Option<String>,
    pub days_of_week: Option<Vec<String>>,
    pub end: Option<String>,
    pub end_date: Option<String>,
    pub end_after: Option<i64>,
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
    pub fn parse(value: Option<&str>) -> Option<Self> {
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

pub fn parse_period_key(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

pub fn period_key_of(date: NaiveDate) -> String {
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

    pub fn rule(&self) -> &RepeatRule {
        &self.rule
    }

    /// Every date of the rule from its start, ending at its end date.
    pub fn dates(&self) -> impl Iterator<Item = NaiveDate> + '_ {
        let end = self.rule.end_date();
        std::iter::successors(Some(self.start), move |date| compute_next_due(date, &self.rule, &self.start))
            .take(MAX_OCCURRENCE_STEPS)
            .take_while(move |date| end.is_none_or(|end| *date <= end))
    }

    pub fn mark(&self, period_key: &str) -> Option<&QuestOccurrence> {
        self.marks.get(period_key).copied()
    }

    pub fn is_closed(&self, period_key: &str) -> bool {
        self.mark(period_key).is_some_and(QuestOccurrence::is_closed)
    }

    fn completed_count(&self) -> i64 {
        self.marks
            .values()
            .filter(|mark| mark.status.as_deref() == Some(OCCURRENCE_COMPLETED))
            .count() as i64
    }

    /// How many more occurrences the rule allows (`after_n`), or no limit.
    pub fn remaining(&self) -> Option<i64> {
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
    pub fn on_day(&self, period_key: &str) -> Quest {
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
    pub fn mark_or_new(&self, period_key: &str) -> QuestOccurrence {
        self.mark(period_key)
            .cloned()
            .unwrap_or_else(|| QuestOccurrence::new(&self.quest.id, period_key, &utc_now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_command_input_distinguishes_omitted_null_and_set_for_clearable_fields() {
        let omitted: UpdateQuestCommandInput =
            serde_json::from_str(r#"{"priority":"high"}"#).expect("omitted keys must deserialize");
        assert_eq!(omitted.due, QuestFieldPatch::Unchanged);
        assert_eq!(omitted.due_time, QuestFieldPatch::Unchanged);
        assert_eq!(omitted.repeat_rule, QuestFieldPatch::Unchanged);
        assert_eq!(omitted.description, QuestFieldPatch::Unchanged);

        let cleared: UpdateQuestCommandInput = serde_json::from_str(
            r#"{"due":null,"due_time":null,"repeat_rule":null,"description":null}"#,
        )
        .expect("explicit null must deserialize");
        assert_eq!(cleared.due, QuestFieldPatch::Clear);
        assert_eq!(cleared.due_time, QuestFieldPatch::Clear);
        assert_eq!(cleared.repeat_rule, QuestFieldPatch::Clear);
        assert_eq!(cleared.description, QuestFieldPatch::Clear);

        let set: UpdateQuestCommandInput = serde_json::from_str(
            r#"{"due":"2026-05-01","due_time":"09:00","repeat_rule":"{\"preset\":\"daily\"}"}"#,
        )
        .expect("explicit values must deserialize");
        assert_eq!(set.due, QuestFieldPatch::Set("2026-05-01".to_string()));
        assert_eq!(set.due_time, QuestFieldPatch::Set("09:00".to_string()));
        assert_eq!(
            set.repeat_rule,
            QuestFieldPatch::Set(r#"{"preset":"daily"}"#.to_string())
        );
    }

    #[test]
    fn create_input_accepts_named_metadata_and_rejects_numeric_priority() {
        let input: CreateQuestInput = serde_json::from_str(
            r#"{"title":"Metadata","description":null,"energy":"large","priority":"high","due":null,"due_time":null,"repeat_rule":null,"order_rank":null}"#,
        )
        .expect("named metadata must deserialize");
        assert_eq!(input.energy, ENERGY_LARGE);
        assert_eq!(input.priority, PRIORITY_HIGH);

        let err = match serde_json::from_str::<CreateQuestInput>(
            r#"{"title":"Metadata","description":null,"energy":"small","priority":3,"due":null,"due_time":null,"repeat_rule":null,"order_rank":null}"#,
        ) {
            Ok(_) => panic!("numeric priority must be rejected at the adapter boundary"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("invalid type"));
    }
}
