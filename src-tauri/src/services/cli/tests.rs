use super::*;
use crate::services::db::temp_db_path;
use clap::CommandFactory;
use std::path::Path;

fn assert_not_raw_json(text: &str) {
    let trimmed = text.trim_start();
    assert!(
        !trimmed.starts_with('{') && !trimmed.starts_with('['),
        "default CLI output must be human-readable, got: {text}"
    );
}

fn seed_unknown_schema_migration_db(db_path: &Path) {
    let mut conn = SqliteConnection::establish(db_path.to_str().expect("valid temp db path"))
        .expect("open db path");

    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS __diesel_schema_migrations (
                version VARCHAR(50) PRIMARY KEY NOT NULL,
                run_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
    )
    .execute(&mut conn)
    .expect("create migrations metadata table");
    diesel::sql_query("INSERT INTO __diesel_schema_migrations (version) VALUES ('99999999999999')")
        .execute(&mut conn)
        .expect("seed unknown future migration version");
}

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn quest_metadata_parser_accepts_named_values_uses_create_defaults_and_rejects_invalid_values() {
    let create_default = Cli::try_parse_from(["fini", "quest", "create", "--title", "Default"])
        .expect("parse create defaults");
    let create_named = Cli::try_parse_from([
        "fini",
        "quest",
        "create",
        "--title",
        "Named",
        "--energy",
        "large",
        "--priority",
        "high",
    ])
    .expect("parse named create metadata");
    let update_named = Cli::try_parse_from([
        "fini",
        "quest",
        "update",
        "--id",
        "quest-1",
        "--energy",
        "small",
        "--priority",
        "low",
    ])
    .expect("parse named update metadata");

    match create_default.command {
        Some(Command::Quest {
            command: QuestCommand::Create(args),
        }) => {
            assert_eq!(args.energy, "medium");
            assert_eq!(args.priority, "medium");
        }
        _ => panic!("expected quest create defaults"),
    }
    match create_named.command {
        Some(Command::Quest {
            command: QuestCommand::Create(args),
        }) => {
            assert_eq!(args.energy, "large");
            assert_eq!(args.priority, "high");
        }
        _ => panic!("expected named quest create"),
    }
    match update_named.command {
        Some(Command::Quest {
            command: QuestCommand::Update(args),
        }) => {
            assert_eq!(args.energy.as_deref(), Some("small"));
            assert_eq!(args.priority.as_deref(), Some("low"));
        }
        _ => panic!("expected named quest update"),
    }

    for command in [
        vec![
            "fini", "quest", "create", "--title", "Bad", "--energy", "tiny",
        ],
        vec![
            "fini",
            "quest",
            "create",
            "--title",
            "Bad",
            "--priority",
            "urgent",
        ],
        vec![
            "fini", "quest", "update", "--id", "quest-1", "--energy", "high",
        ],
        vec![
            "fini",
            "quest",
            "update",
            "--id",
            "quest-1",
            "--priority",
            "4",
        ],
    ] {
        assert!(Cli::try_parse_from(command).is_err());
    }
}

#[test]
fn quest_metadata_handler_serializes_named_values_for_create_and_update() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let db_path = temp_db_path("cli-quest-metadata-handler");
    std::env::set_var("FINI_DB_PATH", &db_path);

    let ctx = CliContext::new()
        .unwrap_or_else(|error| panic!("initialize isolated CLI database: {}", error.message));
    let create = Cli::try_parse_from([
        "fini",
        "quest",
        "create",
        "--title",
        "Ship release",
        "--energy",
        "large",
        "--priority",
        "high",
    ])
    .expect("parse metadata create");
    let create_command = match create.command {
        Some(Command::Quest { command }) => command,
        _ => panic!("expected quest create command"),
    };
    let created = handle_quest(&ctx, create_command)
        .unwrap_or_else(|error| panic!("create quest with metadata: {}", error.message));
    assert_eq!(created["energy"], "large");
    assert_eq!(created["priority"], "high");
    let id = created["id"].as_str().expect("created id").to_string();

    let update = Cli::try_parse_from([
        "fini",
        "quest",
        "update",
        "--id",
        &id,
        "--energy",
        "small",
        "--priority",
        "low",
    ])
    .expect("parse metadata update");
    let update_command = match update.command {
        Some(Command::Quest { command }) => command,
        _ => panic!("expected quest update command"),
    };
    let updated = handle_quest(&ctx, update_command)
        .unwrap_or_else(|error| panic!("update quest metadata: {}", error.message));
    assert_eq!(updated["energy"], "small");
    assert_eq!(updated["priority"], "low");

    std::env::remove_var("FINI_DB_PATH");
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn quest_metadata_help_lists_named_values_defaults_and_update_preservation() {
    let mut create = Cli::command();
    let create_help = create
        .find_subcommand_mut("quest")
        .expect("quest command")
        .find_subcommand_mut("create")
        .expect("quest create command")
        .render_long_help()
        .to_string();
    assert!(create_help.contains("--energy <ENERGY>"));
    assert!(create_help.contains("[default: medium]"));
    assert!(create_help.contains("small"));
    assert!(create_help.contains("large"));
    assert!(create_help.contains("--priority <PRIORITY>"));
    assert!(create_help.contains("low"));
    assert!(create_help.contains("high"));

    let mut update = Cli::command();
    let update_help = update
        .find_subcommand_mut("quest")
        .expect("quest command")
        .find_subcommand_mut("update")
        .expect("quest update command")
        .render_long_help()
        .to_string();
    assert!(update_help.contains("--energy <ENERGY>"));
    assert!(update_help.contains("--priority <PRIORITY>"));
    assert!(update_help.contains("omit to preserve"));
}

#[test]
fn nullable_update_argument_distinguishes_omitted_text_empty_and_literal_null() {
    use crate::models::QuestFieldPatch;

    assert_eq!(parse_nullable_quest_patch(None), QuestFieldPatch::Unchanged);
    assert_eq!(
        parse_nullable_quest_patch(Some("notes".to_string())),
        QuestFieldPatch::Set("notes".to_string())
    );
    assert_eq!(
        parse_nullable_quest_patch(Some(String::new())),
        QuestFieldPatch::Set(String::new())
    );
    assert_eq!(
        parse_nullable_quest_patch(Some("null".to_string())),
        QuestFieldPatch::Clear
    );
}

#[test]
fn repeat_alias_parser_accepts_only_supported_presets_for_create_and_update() {
    for command in [
        vec!["fini", "quest", "create", "--title", "Pay rent"],
        vec!["fini", "quest", "update", "--id", "quest-1"],
    ] {
        for repeat in ["daily", "weekdays", "weekly", "monthly", "yearly", "null"] {
            let mut args = command.clone();
            args.extend(["--repeat", repeat]);
            Cli::try_parse_from(args)
                .unwrap_or_else(|err| panic!("parse --repeat {repeat} for {}: {err}", command[2]));
        }

        let mut invalid = command;
        invalid.extend(["--repeat", "hourly"]);
        assert!(
            Cli::try_parse_from(invalid).is_err(),
            "unsupported repeat aliases must fail"
        );
    }
}

#[test]
fn repeat_alias_normalizes_presets_to_canonical_repeat_rules() {
    for (alias, expected) in [
        ("daily", r#"{"preset":"daily"}"#),
        ("weekdays", r#"{"preset":"weekdays"}"#),
        ("weekly", r#"{"preset":"weekly"}"#),
        ("monthly", r#"{"preset":"monthly"}"#),
        ("yearly", r#"{"preset":"yearly"}"#),
    ] {
        assert_eq!(normalize_repeat_alias(alias), expected);
    }
    assert_eq!(normalize_repeat_alias("null"), "null");
}

#[test]
fn create_repeat_null_creates_a_non_repeating_quest_without_a_series() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let db_path = temp_db_path("cli-create-repeat-null");
    std::env::set_var("FINI_DB_PATH", &db_path);

    let ctx = CliContext::new()
        .unwrap_or_else(|err| panic!("initialize isolated CLI database: {}", err.message));
    let cli = Cli::try_parse_from([
        "fini",
        "quest",
        "create",
        "--title",
        "One-time task",
        "--repeat",
        "null",
    ])
    .expect("parse quest create --repeat null");
    let command = match cli.command {
        Some(Command::Quest { command }) => command,
        _ => panic!("expected quest create command"),
    };

    let created = handle_quest(&ctx, command)
        .unwrap_or_else(|err| panic!("create non-repeating quest: {}", err.message));
    assert_eq!(created["repeat_rule"], Value::Null);

    let mut conn = ctx
        .open_db()
        .unwrap_or_else(|err| panic!("open isolated CLI database: {}", err.message));
    let marks: i64 = crate::schema::quest_occurrences::table
        .count()
        .get_result(&mut conn)
        .expect("count occurrence marks");
    assert_eq!(
        marks, 0,
        "--repeat null makes a one-time quest, with no day rows"
    );

    std::env::remove_var("FINI_DB_PATH");
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn quest_create_normalizes_raw_cli_checklist_before_saving() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let db_path = temp_db_path("cli-create-normalizes-raw-checklist-text");
    std::env::set_var("FINI_DB_PATH", &db_path);

    let ctx = CliContext::new()
        .unwrap_or_else(|err| panic!("initialize isolated CLI database: {}", err.message));
    let create_cli = Cli::try_parse_from([
        "fini",
        "quest",
        "create",
        "--title",
        "Pack bag",
        "--checklist=- [ ] headphones",
    ])
    .expect("parse checklist create");
    let create_command = match create_cli.command {
        Some(Command::Quest { command }) => command,
        _ => panic!("expected quest create command"),
    };
    let created = handle_quest(&ctx, create_command)
        .unwrap_or_else(|err| panic!("create checklist quest: {}", err.message));
    let quest_id = created["id"]
        .as_str()
        .expect("created quest id")
        .to_string();

    let mut conn = ctx
        .open_db()
        .unwrap_or_else(|err| panic!("open isolated CLI database: {}", err.message));
    let stored_quest = QuestRepository::new(&mut conn)
        .get(&quest_id)
        .expect("load created checklist quest");
    let stored_checklist = stored_quest
        .description
        .as_deref()
        .expect("created checklist text");
    let items = crate::services::checklist::parse(stored_checklist);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].text, "headphones");
    assert!(
        stored_checklist.contains("<!--k=") && stored_checklist.ends_with("-->"),
        "stored checklist text must persist hidden item ids, got: {stored_checklist}"
    );

    let listed_item_id = items[0].id.clone();
    let check_cli = Cli::try_parse_from([
        "fini",
        "quest",
        "checklist-check",
        "--quest-id",
        &quest_id,
        "--item-id",
        &listed_item_id,
    ])
    .expect("parse checklist check");
    let check_command = match check_cli.command {
        Some(Command::Quest { command }) => command,
        _ => panic!("expected checklist-check command"),
    };
    let checked = handle_quest(&ctx, check_command)
        .unwrap_or_else(|err| panic!("check listed checklist item: {}", err.message));
    let checked_items = crate::services::checklist::parse(
        checked["description"]
            .as_str()
            .expect("checked checklist text"),
    );
    assert_eq!(checked_items[0].id, listed_item_id);
    assert!(checked_items[0].checked);

    std::env::remove_var("FINI_DB_PATH");
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn checklist_set_template_normalizes_raw_cli_input_before_saving() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let db_path = temp_db_path("cli-checklist-set-template-normalizes-raw-input");
    std::env::set_var("FINI_DB_PATH", &db_path);

    let ctx = CliContext::new()
        .unwrap_or_else(|err| panic!("initialize isolated CLI database: {}", err.message));
    let create_cli = Cli::try_parse_from([
        "fini", "quest", "create", "--title", "Pack bag", "--repeat", "daily", "--item", "old item",
    ])
    .expect("parse recurring checklist create");
    let create_command = match create_cli.command {
        Some(Command::Quest { command }) => command,
        _ => panic!("expected quest create command"),
    };
    let created = handle_quest(&ctx, create_command)
        .unwrap_or_else(|err| panic!("create recurring checklist: {}", err.message));
    let quest_id = created["id"]
        .as_str()
        .expect("created quest id")
        .to_string();

    let set_template_cli = Cli::try_parse_from([
        "fini",
        "quest",
        "checklist-set-template",
        "--quest-id",
        &quest_id,
        "--checklist=- [ ] headphones",
        "--scope",
        "future",
    ])
    .expect("parse checklist-set-template");
    let set_template_command = match set_template_cli.command {
        Some(Command::Quest { command }) => command,
        _ => panic!("expected checklist-set-template command"),
    };
    handle_quest(&ctx, set_template_command)
        .unwrap_or_else(|err| panic!("set checklist template: {}", err.message));

    let mut conn = ctx
        .open_db()
        .unwrap_or_else(|err| panic!("open isolated CLI database: {}", err.message));
    // "future" edits the quest itself, the template of every day (ADR-0012).
    let stored_quest = QuestRepository::new(&mut conn)
        .get(&quest_id)
        .expect("load updated quest");

    {
        let stored_checklist = stored_quest
            .description
            .as_deref()
            .expect("quest checklist md");
        let items = crate::services::checklist::parse(stored_checklist);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "headphones");
        assert!(
            stored_checklist.contains("<!--k=") && stored_checklist.ends_with("-->"),
            "stored checklist text must persist hidden item ids, got: {stored_checklist}"
        );
    }

    std::env::remove_var("FINI_DB_PATH");
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn normalize_future_template_preserves_ids_for_matching_text() {
    let series_template = crate::services::checklist::serialize(&[
        crate::services::checklist::ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: true,
        },
        crate::services::checklist::ChecklistItem {
            id: "a2".into(),
            text: "key fob".into(),
            checked: false,
        },
    ]);

    // Raw, undocumented-id input: "headphones" unchanged, "key fob" dropped, "lunch" added.
    let raw = "- [ ] headphones\n- [ ] lunch";
    let normalized = normalize_future_template(raw, Some(&series_template));
    let items = crate::services::checklist::parse(&normalized);

    assert_eq!(
        items.iter().find(|it| it.text == "headphones").unwrap().id,
        "a1",
        "unchanged text must keep the series template's item id, or reconcile_future_scope \
             can't tell it apart from a brand-new item and resets its check"
    );
    assert_ne!(
        items.iter().find(|it| it.text == "lunch").unwrap().id,
        "a1",
        "a genuinely new line must not collide with an existing id"
    );
    assert_ne!(items.iter().find(|it| it.text == "lunch").unwrap().id, "a2",);
}

#[test]
fn normalize_future_template_matches_the_series_template_not_this_occurrence_only_edits() {
    // The occurrence renamed "headphones" to "wireless headphones" via a "this occurrence"
    // scoped edit — that never touched the series template, which still says "headphones".
    let series_template =
        crate::services::checklist::serialize(&[crate::services::checklist::ChecklistItem {
            id: "a1".into(),
            text: "headphones".into(),
            checked: true,
        }]);
    let occurrence_only_rename =
        crate::services::checklist::serialize(&[crate::services::checklist::ChecklistItem {
            id: "a1".into(),
            text: "wireless headphones".into(),
            checked: true,
        }]);

    // Raw future-template input retypes the *template's* original text.
    let raw = "- [ ] headphones";
    let normalized = normalize_future_template(raw, Some(&series_template));
    let items = crate::services::checklist::parse(&normalized);
    assert_eq!(
        items[0].id, "a1",
        "must match against the series template's own item, not the occurrence's \
             this-occurrence-only rename, or the template's real item gets a fresh id and \
             reconcile_future_scope drops it instead of updating it"
    );

    // Matching against the (stale) occurrence text instead would have missed entirely.
    let normalized_against_occurrence =
        normalize_future_template(raw, Some(&occurrence_only_rename));
    let items_against_occurrence =
        crate::services::checklist::parse(&normalized_against_occurrence);
    assert_ne!(
        items_against_occurrence[0].id, "a1",
        "sanity check: matching against the renamed occurrence text does NOT find the id — \
             confirms the series template must be the match source, not the occurrence"
    );
}

#[test]
fn normalize_future_template_assigns_duplicate_text_lines_distinct_existing_ids() {
    let series_template = crate::services::checklist::serialize(&[
        crate::services::checklist::ChecklistItem {
            id: "a1".into(),
            text: "milk".into(),
            checked: false,
        },
        crate::services::checklist::ChecklistItem {
            id: "a2".into(),
            text: "milk".into(),
            checked: true,
        },
    ]);

    let raw = "- [ ] milk\n- [ ] milk";
    let normalized = normalize_future_template(raw, Some(&series_template));
    let items = crate::services::checklist::parse(&normalized);

    assert_eq!(items.len(), 2);
    let ids: std::collections::HashSet<&str> = items.iter().map(|it| it.id.as_str()).collect();
    assert_eq!(
        ids,
        std::collections::HashSet::from(["a1", "a2"]),
        "each duplicate-text line must be paired with a distinct existing id, not share one \
             — a shared id breaks remove-by-id and per-item sync identity for both rows"
    );
}

#[test]
fn cli_checklist_activity_sync_dedupes_per_space() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let db_path = temp_db_path("cli-checklist-toggle-dedupes-activity-sync");
    std::env::set_var("FINI_DB_PATH", &db_path);

    let ctx = CliContext::new()
        .unwrap_or_else(|err| panic!("initialize isolated CLI database: {}", err.message));
    let create_cli = Cli::try_parse_from([
        "fini",
        "quest",
        "create",
        "--title",
        "Pack bag",
        "--item",
        "water bottle",
    ])
    .expect("parse checklist create");
    let create_command = match create_cli.command {
        Some(Command::Quest { command }) => command,
        _ => panic!("expected quest create command"),
    };
    let created = handle_quest(&ctx, create_command)
        .unwrap_or_else(|err| panic!("create checklist quest: {}", err.message));
    let quest_id = created["id"]
        .as_str()
        .expect("created quest id")
        .to_string();

    let add_cli = Cli::try_parse_from([
        "fini",
        "quest",
        "checklist-add",
        "--quest-id",
        &quest_id,
        "--text",
        "headphones",
    ])
    .expect("parse checklist add");
    let add_command = match add_cli.command {
        Some(Command::Quest { command }) => command,
        _ => panic!("expected checklist-add command"),
    };
    let added = handle_quest(&ctx, add_command)
        .unwrap_or_else(|err| panic!("add checklist item: {}", err.message));
    let added_description = added["description"]
        .as_str()
        .expect("updated checklist text");
    let added_item_id = crate::services::checklist::parse(added_description)
        .into_iter()
        .find(|item| item.text == "headphones")
        .expect("added checklist item")
        .id;

    let check_cli = Cli::try_parse_from([
        "fini",
        "quest",
        "checklist-check",
        "--quest-id",
        &quest_id,
        "--item-id",
        &added_item_id,
    ])
    .expect("parse checklist check");
    let check_command = match check_cli.command {
        Some(Command::Quest { command }) => command,
        _ => panic!("expected checklist-check command"),
    };
    handle_quest(&ctx, check_command)
        .unwrap_or_else(|err| panic!("check checklist item: {}", err.message));

    let mut conn = ctx
        .open_db()
        .unwrap_or_else(|err| panic!("open isolated CLI database: {}", err.message));
    let activity_events: Vec<String> = crate::schema::sync_outbox::table
        .filter(crate::schema::sync_outbox::entity_type.eq("checklist_activity"))
        .filter(crate::schema::sync_outbox::op_type.eq("upsert"))
        .select(crate::schema::sync_outbox::entity_id)
        .load(&mut conn)
        .expect("load checklist activity sync events");

    assert_eq!(
        activity_events.len(),
        1,
        "later CLI checklist toggles must not re-emit old checklist_activity rows"
    );

    let space_cli = Cli::try_parse_from(["fini", "space", "create", "--name", "Work"])
        .expect("parse space create");
    let space_command = match space_cli.command {
        Some(Command::Space { command }) => command,
        _ => panic!("expected space create command"),
    };
    let created_space = handle_space(&ctx, space_command)
        .unwrap_or_else(|err| panic!("create destination space: {}", err.message));
    let destination_space_id = created_space["id"]
        .as_str()
        .expect("created space id")
        .to_string();

    let update_cli = Cli::try_parse_from([
        "fini",
        "quest",
        "update",
        "--id",
        &quest_id,
        "--space-id",
        &destination_space_id,
    ])
    .expect("parse checklist space move");
    let update_command = match update_cli.command {
        Some(Command::Quest { command }) => command,
        _ => panic!("expected quest update command"),
    };
    handle_quest(&ctx, update_command)
        .unwrap_or_else(|err| panic!("move checklist quest: {}", err.message));

    let activity_events_by_space: Vec<(String, String)> = crate::schema::sync_outbox::table
        .filter(crate::schema::sync_outbox::entity_type.eq("checklist_activity"))
        .filter(crate::schema::sync_outbox::op_type.eq("upsert"))
        .select((
            crate::schema::sync_outbox::entity_id,
            crate::schema::sync_outbox::space_id,
        ))
        .load(&mut conn)
        .expect("load checklist activity sync event spaces");

    assert_eq!(
        activity_events_by_space,
        vec![
            (activity_events[0].clone(), "1".to_string()),
            (activity_events[0].clone(), destination_space_id),
        ],
        "moving a checklist via the CLI must re-emit prior activity under the destination space"
    );

    std::env::remove_var("FINI_DB_PATH");
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn repeat_alias_conflicts_with_legacy_repeat_rule_and_preserves_update_null_clear() {
    let legacy = Cli::try_parse_from([
        "fini",
        "quest",
        "create",
        "--title",
        "Pay rent",
        "--repeat-rule",
        r#"{"preset":"weekly"}"#,
    ])
    .expect("legacy --repeat-rule remains accepted");
    let legacy_rule = match legacy.command {
        Some(Command::Quest {
            command: QuestCommand::Create(args),
        }) => args.repeat_rule,
        _ => panic!("expected quest create command"),
    };
    assert_eq!(legacy_rule.as_deref(), Some(r#"{"preset":"weekly"}"#));

    let conflict = Cli::try_parse_from([
        "fini",
        "quest",
        "create",
        "--title",
        "Pay rent",
        "--repeat",
        "monthly",
        "--repeat-rule",
        r#"{"preset":"weekly"}"#,
    ]);
    assert!(
        conflict.is_err(),
        "--repeat and --repeat-rule cannot be combined"
    );

    assert_eq!(
        parse_nullable_quest_patch(Some(normalize_repeat_alias("null"))),
        crate::models::QuestFieldPatch::Clear,
        "--repeat null must retain update clear semantics"
    );
}

#[test]
fn export_parser_preserves_repeated_space_ids() {
    let cli = Cli::try_parse_from([
        "fini",
        "export",
        "--path",
        "backup.zip",
        "--space-id",
        "space-a",
        "--space-id",
        "space-b",
    ])
    .expect("parse top-level export");

    let args = match cli.command {
        Some(Command::Export(args)) => args,
        _ => panic!("expected top-level export command"),
    };

    assert_eq!(args.path, "backup.zip");
    assert_eq!(args.space_id, ["space-a", "space-b"]);
    assert!(!args.all_spaces);
}

#[test]
fn top_level_import_inspect_emits_manifest_json_without_mutating_target_db() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let source_db_path = temp_db_path("cli-import-inspect-source");
    let target_db_path = temp_db_path("cli-import-inspect-target");
    let backup_path = source_db_path.with_extension("zip");
    let mut source = crate::services::db::open_db_at_path(&source_db_path);
    diesel::insert_into(crate::schema::spaces::table)
        .values((
            crate::schema::spaces::id.eq("inspect-space"),
            crate::schema::spaces::name.eq("Inspectable space"),
            crate::schema::spaces::item_order.eq(10_i64),
        ))
        .execute(&mut source)
        .expect("seed source database");
    backup::export_backup(&mut source, &backup_path, &["inspect-space".to_string()])
        .expect("create backup archive");
    drop(source);

    let mut target = crate::services::db::open_db_at_path(&target_db_path);
    diesel::insert_into(crate::schema::spaces::table)
        .values((
            crate::schema::spaces::id.eq("target-space"),
            crate::schema::spaces::name.eq("Untouched target space"),
            crate::schema::spaces::item_order.eq(10_i64),
        ))
        .execute(&mut target)
        .expect("seed target database");
    drop(target);
    let target_before =
        std::fs::read(&target_db_path).expect("read target database before inspect");

    std::env::set_var("FINI_DB_PATH", &target_db_path);
    let inspection = handle_import(&BackupInspectArgs {
        path: backup_path.to_string_lossy().into_owned(),
        inspect: true,
        verify: false,
        dry_run: false,
        map_space: vec![],
    })
    .unwrap_or_else(|err| panic!("inspect backup without a local database: {}", err.message));
    let execute_result = execute(Cli {
        json: true,
        command: Some(Command::Import(BackupInspectArgs {
            path: backup_path.to_string_lossy().into_owned(),
            inspect: true,
            verify: false,
            dry_run: false,
            map_space: vec![],
        })),
    });
    std::env::remove_var("FINI_DB_PATH");

    assert_eq!(
        execute_result.unwrap_or_else(|err| panic!("execute import inspect: {}", err.message)),
        EXIT_SUCCESS
    );
    let output = serde_json::to_string(&inspection).expect("serialize inspection as JSON");
    let parsed: Value = serde_json::from_str(&output).expect("inspection output is valid JSON");
    assert_eq!(parsed["valid"], true);
    assert_eq!(parsed["manifest"]["counts"]["spaces"], 1);
    assert_eq!(parsed["manifest"]["spaces"][0]["id"], "inspect-space");
    let target_after = std::fs::read(&target_db_path).expect("read target database after inspect");
    assert_eq!(
        target_after, target_before,
        "inspection must not mutate the target database"
    );

    let _ = std::fs::remove_file(backup_path);
    let _ = std::fs::remove_file(source_db_path);
    let _ = std::fs::remove_file(target_db_path);
}

#[test]
fn top_level_import_verify_parses_repeated_space_mappings() {
    let cli = Cli::try_parse_from([
        "fini",
        "import",
        "--path",
        "backup.zip",
        "--verify",
        "--map-space",
        "backup-new=create_new",
        "--map-space",
        "backup-existing=use_existing:local-space",
    ])
    .expect("parse top-level verify mappings");
    let args = match cli.command {
        Some(Command::Import(args)) => args,
        _ => panic!("expected top-level import command"),
    };
    let mappings = match parse_backup_space_mappings(&args.map_space) {
        Ok(mappings) => mappings,
        Err(err) => panic!("parse repeated space mapping values: {}", err.message),
    };

    assert!(args.verify);
    assert_eq!(mappings.len(), 2);
    assert_eq!(mappings[0].backup_space_id, "backup-new");
    assert_eq!(mappings[0].mode, "create_new");
    assert_eq!(mappings[1].backup_space_id, "backup-existing");
    assert_eq!(mappings[1].mode, "use_existing");
    assert_eq!(mappings[1].local_space_id.as_deref(), Some("local-space"));
}

#[test]
fn top_level_import_verify_rejects_duplicate_backup_space_mappings() {
    let mappings = vec![
        "backup-space=create_new".to_string(),
        "backup-space=use_existing:local-space".to_string(),
    ];

    let err = parse_backup_space_mappings(&mappings)
        .expect_err("duplicate backup space mappings must be rejected");

    assert_eq!(
        err.message,
        "--map-space backup space ID is duplicated: backup-space"
    );
}

#[test]
fn top_level_import_verify_reports_missing_custom_space_mapping_without_mutating_target_db() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let source_db_path = temp_db_path("cli-import-verify-source");
    let target_db_path = temp_db_path("cli-import-verify-target");
    let backup_path = source_db_path.with_extension("zip");
    let mut source = crate::services::db::open_db_at_path(&source_db_path);
    diesel::insert_into(crate::schema::spaces::table)
        .values((
            crate::schema::spaces::id.eq("custom-verify-space"),
            crate::schema::spaces::name.eq("Custom verify space"),
            crate::schema::spaces::item_order.eq(10_i64),
        ))
        .execute(&mut source)
        .expect("seed source database");
    backup::export_backup(
        &mut source,
        &backup_path,
        &["custom-verify-space".to_string()],
    )
    .expect("create backup archive");
    drop(source);

    let mut target = crate::services::db::open_db_at_path(&target_db_path);
    diesel::insert_into(crate::schema::spaces::table)
        .values((
            crate::schema::spaces::id.eq("target-space"),
            crate::schema::spaces::name.eq("Untouched target space"),
            crate::schema::spaces::item_order.eq(10_i64),
        ))
        .execute(&mut target)
        .expect("seed target database");
    drop(target);
    let target_before = std::fs::read(&target_db_path).expect("read target database before verify");

    std::env::set_var("FINI_DB_PATH", &target_db_path);
    let verification = handle_import(&BackupInspectArgs {
        path: backup_path.to_string_lossy().into_owned(),
        inspect: false,
        verify: true,
        dry_run: false,
        map_space: vec![],
    })
    .unwrap_or_else(|err| panic!("verify backup: {}", err.message));
    std::env::remove_var("FINI_DB_PATH");

    assert_eq!(
        verification["manifest"]["spaces"][0]["id"],
        "custom-verify-space"
    );
    assert_eq!(
        verification["required_space_mappings"][0]["backup_space_id"],
        "custom-verify-space"
    );
    assert_eq!(verification["conflicts"], json!([]));
    let target_after = std::fs::read(&target_db_path).expect("read target database after verify");
    assert_eq!(
        target_after, target_before,
        "verification must not mutate the target database"
    );

    let _ = std::fs::remove_file(backup_path);
    let _ = std::fs::remove_file(source_db_path);
    let _ = std::fs::remove_file(target_db_path);
}

#[test]
fn top_level_import_dry_run_emits_a_non_mutating_ready_plan() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let source_db_path = temp_db_path("cli-import-dry-run-source");
    let target_db_path = temp_db_path("cli-import-dry-run-target");
    let backup_path = source_db_path.with_extension("zip");
    let mut source = crate::services::db::open_db_at_path(&source_db_path);
    diesel::insert_into(crate::schema::spaces::table)
        .values((
            crate::schema::spaces::id.eq("dry-run-space"),
            crate::schema::spaces::name.eq("Dry run space"),
            crate::schema::spaces::item_order.eq(10_i64),
        ))
        .execute(&mut source)
        .expect("seed source database");
    backup::export_backup(&mut source, &backup_path, &["dry-run-space".to_string()])
        .expect("create backup archive");
    drop(source);

    let target = crate::services::db::open_db_at_path(&target_db_path);
    drop(target);
    let target_before =
        std::fs::read(&target_db_path).expect("read target database before dry run");

    std::env::set_var("FINI_DB_PATH", &target_db_path);
    let plan = handle_import(&BackupInspectArgs {
        path: backup_path.to_string_lossy().into_owned(),
        inspect: false,
        verify: false,
        dry_run: true,
        map_space: vec!["dry-run-space=create_new".to_string()],
    })
    .unwrap_or_else(|err| panic!("dry run backup: {}", err.message));
    std::env::remove_var("FINI_DB_PATH");

    assert_eq!(plan["dry_run"], true);
    assert_eq!(plan["ready_to_apply"], true);
    assert_eq!(plan["no_apply_or_recovery_action_occurred"], true);
    assert_eq!(plan["manifest"]["spaces"][0]["id"], "dry-run-space");
    assert_eq!(plan["required_space_mappings"], json!([]));
    assert_eq!(plan["conflicts"], json!([]));

    std::env::set_var("FINI_DB_PATH", &target_db_path);
    let exit_code = execute(Cli {
        json: true,
        command: Some(Command::Import(BackupInspectArgs {
            path: backup_path.to_string_lossy().into_owned(),
            inspect: false,
            verify: false,
            dry_run: true,
            map_space: vec!["dry-run-space=create_new".to_string()],
        })),
    })
    .unwrap_or_else(|err| panic!("execute dry run: {}", err.message));
    std::env::remove_var("FINI_DB_PATH");
    assert_eq!(exit_code, EXIT_SUCCESS);
    assert_eq!(
        std::fs::read(&target_db_path).expect("read target database after dry run"),
        target_before,
        "dry run must not mutate the target database"
    );

    let _ = std::fs::remove_file(backup_path);
    let _ = std::fs::remove_file(source_db_path);
    let _ = std::fs::remove_file(target_db_path);
}

#[test]
fn top_level_import_dry_run_reports_unresolved_mapping_as_not_ready() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let source_db_path = temp_db_path("cli-import-dry-run-unresolved-source");
    let target_db_path = temp_db_path("cli-import-dry-run-unresolved-target");
    let backup_path = source_db_path.with_extension("zip");
    let mut source = crate::services::db::open_db_at_path(&source_db_path);
    diesel::insert_into(crate::schema::spaces::table)
        .values((
            crate::schema::spaces::id.eq("unmapped-dry-run-space"),
            crate::schema::spaces::name.eq("Unmapped dry run space"),
            crate::schema::spaces::item_order.eq(10_i64),
        ))
        .execute(&mut source)
        .expect("seed source database");
    backup::export_backup(
        &mut source,
        &backup_path,
        &["unmapped-dry-run-space".to_string()],
    )
    .expect("create backup archive");
    drop(source);
    let target = crate::services::db::open_db_at_path(&target_db_path);
    drop(target);

    std::env::set_var("FINI_DB_PATH", &target_db_path);
    let plan = handle_import(&BackupInspectArgs {
        path: backup_path.to_string_lossy().into_owned(),
        inspect: false,
        verify: false,
        dry_run: true,
        map_space: vec![],
    })
    .unwrap_or_else(|err| panic!("dry run backup: {}", err.message));
    std::env::remove_var("FINI_DB_PATH");

    assert_eq!(plan["dry_run"], true);
    assert_eq!(plan["ready_to_apply"], false);
    assert_eq!(plan["no_apply_or_recovery_action_occurred"], true);
    assert_eq!(
        plan["required_space_mappings"][0]["backup_space_id"],
        "unmapped-dry-run-space"
    );
    assert_eq!(plan["conflicts"], json!([]));

    let _ = std::fs::remove_file(backup_path);
    let _ = std::fs::remove_file(source_db_path);
    let _ = std::fs::remove_file(target_db_path);
}

#[test]
fn top_level_import_requires_read_only_mode_and_rejects_incompatible_or_unsupported_flags() {
    let path_only = Cli::try_parse_from(["fini", "import", "--path", "backup.zip"])
        .expect("path-only import parses so execution can explain the unavailable mutation mode");
    let err = execute(path_only).expect_err("path-only import must be rejected");
    assert_eq!(
        err.message,
        "import requires --inspect, --verify, or --dry-run; mutation modes are not available yet"
    );

    let incompatible = Cli::try_parse_from([
        "fini",
        "import",
        "--path",
        "backup.zip",
        "--inspect",
        "--verify",
    ]);
    assert!(
        incompatible.is_err(),
        "import must reject combining inspect with verify"
    );

    let dry_run_with_mappings = Cli::try_parse_from([
        "fini",
        "import",
        "--path",
        "backup.zip",
        "--dry-run",
        "--map-space",
        "backup-one=create_new",
        "--map-space",
        "backup-two=use_existing:local-space",
    ]);
    assert!(
        dry_run_with_mappings.is_ok(),
        "dry run must allow repeated space mappings"
    );

    let inspect_with_mappings = Cli::try_parse_from([
        "fini",
        "import",
        "--path",
        "backup.zip",
        "--inspect",
        "--map-space",
        "backup-one=create_new",
    ]);
    assert!(
        inspect_with_mappings.is_err(),
        "space mappings must only be accepted for verify or dry run"
    );

    let unsupported = Cli::try_parse_from([
        "fini",
        "import",
        "--path",
        "backup.zip",
        "--verify",
        "--force",
    ]);
    assert!(
        unsupported.is_err(),
        "import must reject unsupported mutation flags"
    );
}

#[test]
fn top_level_export_rejects_missing_scope() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let db_path = temp_db_path("cli-export-requires-scope");
    std::env::set_var("FINI_DB_PATH", &db_path);

    let result = execute(Cli {
        json: true,
        command: Some(Command::Export(BackupExportArgs {
            path: db_path.with_extension("zip").to_string_lossy().into_owned(),
            space_id: vec![],
            all_spaces: false,
        })),
    });

    std::env::remove_var("FINI_DB_PATH");
    let _ = std::fs::remove_file(&db_path);

    let err = result.expect_err("export without a scope must fail");
    assert_eq!(err.code, EXIT_RUNTIME);
    assert_eq!(err.message, "export requires --space-id or --all-spaces");
}

#[test]
fn top_level_export_accepts_all_spaces_and_writes_json_result() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let db_path = temp_db_path("cli-export-all-spaces");
    let export_path = db_path.with_extension("zip");
    std::env::set_var("FINI_DB_PATH", &db_path);

    let ctx = match CliContext::new() {
        Ok(ctx) => ctx,
        Err(err) => panic!("initialize isolated CLI database: {}", err.message),
    };
    handle_space(
        &ctx,
        SpaceCommand::Create(SpaceCreateArgs {
            name: "Exported space".to_string(),
        }),
    )
    .unwrap_or_else(|err| panic!("seed isolated database with a space: {}", err.message));

    let result = execute(Cli {
        json: true,
        command: Some(Command::Export(BackupExportArgs {
            path: export_path.to_string_lossy().into_owned(),
            space_id: vec![],
            all_spaces: true,
        })),
    });

    std::env::remove_var("FINI_DB_PATH");
    let _ = std::fs::remove_file(&db_path);

    assert_eq!(
        result.unwrap_or_else(|err| panic!("export all spaces: {}", err.message)),
        EXIT_SUCCESS
    );
    assert!(export_path.exists(), "export should write a backup archive");
    let _ = std::fs::remove_file(export_path);
}

#[test]
fn cli_help_describes_json_as_structured_opt_in() {
    let help = Cli::command().render_help().to_string();

    assert!(help.contains("--json"));
    assert!(help.contains("Opt into structured JSON output for automation"));
}

#[test]
fn default_focus_output_is_human_readable_when_no_quest_is_active() {
    let output = format_human_output(&Value::Null);

    assert_eq!(output, "No active Focus quest.");
    assert_not_raw_json(&output);
}

#[test]
fn representative_default_outputs_are_human_readable() {
    let quest_list = format_human_output(&json!({
        "quests": [{
            "id": "quest-1",
            "title": "Write issue regression",
            "status": "active"
        }]
    }));
    let space_list = format_human_output(&json!({
        "spaces": [{
            "id": "1",
            "name": "Personal",
            "item_order": 0
        }]
    }));
    let generic_object_fallback = format_human_output(&json!({
        "sent_events": 1,
        "applied_events": 0,
        "received_acks": 2,
        "peers": [{ "peer_device_id": "peer-1" }],
        "nested": { "ok": true }
    }));
    let generic_array_fallback = format_human_output(&json!([{
        "request_id": "request-1",
        "from_device_id": "device-1",
        "payload": { "code": "123456" }
    }]));

    assert_eq!(quest_list, "- [active] Write issue regression (quest-1)");
    assert_eq!(space_list, "- Personal (1)");
    assert!(generic_object_fallback.contains("Result:"));
    assert!(generic_array_fallback.contains("Result:"));
    for output in [
        quest_list,
        space_list,
        generic_object_fallback,
        generic_array_fallback,
    ] {
        assert_not_raw_json(&output);
        assert!(
            !output.contains("\n{"),
            "must not print pretty JSON: {output}"
        );
    }
}

#[test]
fn json_output_path_still_serializes_structured_json() {
    let value = json!({
        "quests": [{
            "id": "quest-1",
            "title": "Keep automation structured",
            "status": "active"
        }]
    });
    let text = serde_json::to_string_pretty(&value).expect("serialize JSON output");

    assert!(serde_json::from_str::<Value>(&text).is_ok());
    assert!(text.trim_start().starts_with('{'));
}

#[test]
fn cli_auto_update_is_disabled_for_debug_test_builds() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    std::env::remove_var("FINI_DISABLE_AUTO_UPDATE");

    assert!(!should_run_auto_update());
}

#[test]
fn cli_auto_update_respects_disable_env() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    std::env::set_var("FINI_DISABLE_AUTO_UPDATE", "1");

    assert!(!should_run_auto_update());

    std::env::remove_var("FINI_DISABLE_AUTO_UPDATE");
}

#[test]
fn cli_execute_rejects_unknown_schema_migration_before_device_state_creation() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let db_path = temp_db_path("cli-db-open-rejects-unknown-schema-migration");
    seed_unknown_schema_migration_db(&db_path);
    std::env::set_var("FINI_DB_PATH", &db_path);
    std::env::set_var("FINI_INSTALL_CHANNEL", "appimage");

    let result = execute(Cli {
        json: true,
        command: Some(Command::Space {
            command: SpaceCommand::List,
        }),
    });
    std::env::remove_var("FINI_DB_PATH");
    std::env::remove_var("FINI_INSTALL_CHANNEL");

    let err = match result {
        Ok(_) => panic!("CLI execute must reject database with unknown migration"),
        Err(err) => err,
    };

    assert_eq!(err.code, EXIT_RUNTIME);
    assert!(
        err.message
            .contains("database schema is not supported by this Fini binary"),
        "error should explain unsupported schema, got: {}",
        err.message
    );
    assert!(
        err.message.contains("Update required (AppImage)"),
        "error should include next step, got: {}",
        err.message
    );
    assert!(
        err.message.contains("latest Fini AppImage"),
        "error should include AppImage guidance, got: {}",
        err.message
    );

    let _ = std::fs::remove_file(db_path);
}
