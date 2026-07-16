use std::fs;
use std::path::Path;

use chrono::DateTime;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationImportQuest {
    pub title: String,
    pub description: Option<String>,
    pub space_id: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationImportRecord {
    title: String,
    description: Option<String>,
    space_id: String,
    status: String,
    created_at: String,
    updated_at: String,
    completed_at: Option<String>,
}

pub fn parse_migration_import_file(
    path: impl AsRef<Path>,
) -> Result<Vec<MigrationImportQuest>, String> {
    let payload = fs::read_to_string(path.as_ref())
        .map_err(|error| format!("failed to read migration import file: {error}"))?;
    parse_migration_import_json(&payload)
}

pub fn parse_migration_import_json(payload: &str) -> Result<Vec<MigrationImportQuest>, String> {
    let records: Vec<MigrationImportRecord> =
        serde_json::from_str(payload).map_err(|error| format!("invalid JSON: {error}"))?;

    records
        .into_iter()
        .enumerate()
        .map(|(index, record)| validate_record(index, record))
        .collect()
}

fn validate_record(
    index: usize,
    record: MigrationImportRecord,
) -> Result<MigrationImportQuest, String> {
    let position = index + 1;
    validate_non_empty(position, "title", &record.title)?;
    validate_non_empty(position, "space_id", &record.space_id)?;
    validate_timestamp(position, "created_at", &record.created_at)?;
    validate_timestamp(position, "updated_at", &record.updated_at)?;
    if let Some(completed_at) = &record.completed_at {
        validate_timestamp(position, "completed_at", completed_at)?;
    }

    Ok(MigrationImportQuest {
        title: record.title,
        description: record.description,
        space_id: record.space_id,
        status: record.status,
        created_at: record.created_at,
        updated_at: record.updated_at,
        completed_at: record.completed_at,
    })
}

fn validate_non_empty(index: usize, field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("record {index}: {field} must not be empty"));
    }
    Ok(())
}

fn validate_timestamp(index: usize, field: &str, value: &str) -> Result<(), String> {
    DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| format!("record {index}: {field} must be RFC3339"))
}

#[cfg(test)]
mod tests {
    use super::{parse_migration_import_file, parse_migration_import_json};
    use std::fs;

    const VALID_IMPORT: &str = r#"
        [{
            "title": "Historic quest",
            "description": "Imported from a prior system",
            "space_id": "space-archive",
            "status": "completed",
            "created_at": "2019-04-12T08:15:30+02:00",
            "updated_at": "2021-11-05T14:45:00Z",
            "completed_at": "2021-11-06T09:30:00-05:00"
        }]
    "#;

    #[test]
    fn parses_historical_timestamps_and_preserves_original_values() {
        let imported = parse_migration_import_json(VALID_IMPORT).expect("valid migration import");

        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].title, "Historic quest");
        assert_eq!(
            imported[0].description.as_deref(),
            Some("Imported from a prior system")
        );
        assert_eq!(imported[0].space_id, "space-archive");
        assert_eq!(imported[0].status, "completed");
        assert_eq!(imported[0].created_at, "2019-04-12T08:15:30+02:00");
        assert_eq!(imported[0].updated_at, "2021-11-05T14:45:00Z");
        assert_eq!(
            imported[0].completed_at.as_deref(),
            Some("2021-11-06T09:30:00-05:00")
        );
    }

    #[test]
    fn parses_a_json_file_payload() {
        let path = std::env::temp_dir().join(format!(
            "fini-migration-import-{}-{}.json",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::write(&path, VALID_IMPORT).expect("write test import");

        let imported = parse_migration_import_file(&path).expect("valid migration import file");

        fs::remove_file(path).expect("remove test import");
        assert_eq!(imported.len(), 1);
    }

    #[test]
    fn accepts_missing_optional_description_and_completed_at() {
        let imported = parse_migration_import_json(
            r#"[{"title":"Open quest","space_id":"space-1","status":"open","created_at":"2020-01-01T00:00:00Z","updated_at":"2020-01-01T00:00:00Z"}]"#,
        )
        .expect("optional fields may be absent");

        assert_eq!(imported[0].description, None);
        assert_eq!(imported[0].completed_at, None);
    }

    #[test]
    fn rejects_unknown_fields() {
        let error = parse_migration_import_json(
            r#"[{"title":"Quest","space_id":"space-1","status":"open","created_at":"2020-01-01T00:00:00Z","updated_at":"2020-01-01T00:00:00Z","unexpected":true}]"#,
        )
        .expect_err("unknown fields must be rejected");

        assert!(error.contains("unknown field"), "unexpected error: {error}");
    }

    #[test]
    fn rejects_malformed_json() {
        let error =
            parse_migration_import_json("[{\"title\":]").expect_err("malformed JSON must fail");

        assert!(error.contains("invalid JSON"), "unexpected error: {error}");
    }

    #[test]
    fn rejects_empty_titles() {
        let error = parse_migration_import_json(
            r#"[{"title":"   ","space_id":"space-1","status":"open","created_at":"2020-01-01T00:00:00Z","updated_at":"2020-01-01T00:00:00Z"}]"#,
        )
        .expect_err("empty title must fail");

        assert!(
            error.contains("title must not be empty"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_empty_space_ids() {
        let error = parse_migration_import_json(
            r#"[{"title":"Quest","space_id":"","status":"open","created_at":"2020-01-01T00:00:00Z","updated_at":"2020-01-01T00:00:00Z"}]"#,
        )
        .expect_err("empty space ID must fail");

        assert!(
            error.contains("space_id must not be empty"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_invalid_created_at_timestamp() {
        let error = parse_migration_import_json(
            r#"[{"title":"Quest","space_id":"space-1","status":"open","created_at":"not-a-timestamp","updated_at":"2020-01-01T00:00:00Z"}]"#,
        )
        .expect_err("invalid created_at must fail");

        assert!(
            error.contains("created_at must be RFC3339"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_invalid_updated_at_timestamp() {
        let error = parse_migration_import_json(
            r#"[{"title":"Quest","space_id":"space-1","status":"open","created_at":"2020-01-01T00:00:00Z","updated_at":"not-a-timestamp"}]"#,
        )
        .expect_err("invalid updated_at must fail");

        assert!(
            error.contains("updated_at must be RFC3339"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_invalid_completed_at_timestamp() {
        let error = parse_migration_import_json(
            r#"[{"title":"Quest","space_id":"space-1","status":"completed","created_at":"2020-01-01T00:00:00Z","updated_at":"2020-01-01T00:00:00Z","completed_at":"not-a-timestamp"}]"#,
        )
        .expect_err("invalid completed_at must fail");

        assert!(
            error.contains("completed_at must be RFC3339"),
            "unexpected error: {error}"
        );
    }
}
