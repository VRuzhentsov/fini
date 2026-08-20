use diesel::prelude::*;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::schema::quests;

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
    pub series_id: Option<String>,
    pub period_key: Option<String>,
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

impl QuestUpdatePatch {
    #[cfg(test)]
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
