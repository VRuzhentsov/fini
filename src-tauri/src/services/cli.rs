use clap::{ArgAction, Args, Parser, Subcommand};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde_json::{json, Value};

use crate::models::{
    CreateQuestInput, CreateReminderInput, CreateSpaceInput, UpdateQuestInput, UpdateSpaceInput,
};
use crate::schema::quests;
use crate::services::backup;
use crate::services::cli_update::{maybe_auto_update, run_update, UpdateOptions};
use crate::services::db::{db_default_path, try_open_db_at_path, utc_now};
use crate::services::device_connection::types::{
    DevicePairRequestAckInput, DevicePairRequestInput,
};
use crate::services::device_connection::{
    device_connection_consume_space_mapping_updates_impl, device_connection_debug_status_impl,
    device_connection_discovery_snapshot_impl, device_connection_enter_add_mode_impl,
    device_connection_get_identity_impl, device_connection_get_paired_devices_impl,
    device_connection_leave_add_mode_impl, device_connection_pair_accept_request_impl,
    device_connection_pair_acknowledge_request_impl, device_connection_pair_complete_request_impl,
    device_connection_pair_incoming_requests_impl,
    device_connection_pair_outgoing_completions_impl, device_connection_pair_outgoing_updates_impl,
    device_connection_presence_snapshot_impl, device_connection_save_paired_device_impl,
    device_connection_send_pair_request_impl, device_connection_unpair_impl,
    device_connection_update_last_seen_impl, DeviceConnectionState,
};
use crate::services::quest::{
    append_focus_history, record_focus_enter, resolve_active_quest, QuestRepository,
};
use crate::services::reminder::ReminderRepository;
use crate::services::settings::{self, ThemeMode};
use crate::services::space::SpaceRepository;
use crate::services::space_sync::outbox::emit_sync_event;
use crate::services::space_sync::{
    space_sync_apply_remote_mappings_impl, space_sync_list_mappings_impl,
    space_sync_resolve_custom_space_mapping_impl, space_sync_status_impl, space_sync_tick_impl,
    space_sync_update_mappings_impl, SpaceResolutionMode,
};

const EXIT_SUCCESS: i32 = 0;
const EXIT_NOT_FOUND: i32 = 3;
const EXIT_INVALID_STATE: i32 = 4;
const EXIT_RUNTIME: i32 = 5;

#[derive(Parser)]
#[command(name = "fini", version, about = "Fini CLI")]
pub struct Cli {
    #[arg(
        long,
        global = true,
        help = "Opt into structured JSON output for automation"
    )]
    json: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Download and install the latest standalone Fini CLI release")]
    Update(UpdateArgs),
    Focus {
        #[command(subcommand)]
        command: FocusCommand,
    },
    Quest {
        #[command(subcommand)]
        command: QuestCommand,
    },
    Space {
        #[command(subcommand)]
        command: SpaceCommand,
    },
    Backup {
        #[command(subcommand)]
        command: BackupCommand,
    },
    Reminder {
        #[command(subcommand)]
        command: ReminderCommand,
    },
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    Sync {
        #[command(subcommand)]
        command: SyncCommand,
    },
    Settings {
        #[command(subcommand)]
        command: SettingsCommand,
    },
}

#[derive(Args)]
struct UpdateArgs {
    #[arg(
        long,
        help = "Print the selected release and install paths without changing files"
    )]
    dry_run: bool,
    #[arg(
        long,
        help = "Override the Tauri updater endpoint; defaults to FINI_UPDATE_ENDPOINT or Fini's latest CLI manifest"
    )]
    endpoint: Option<String>,
    #[arg(
        long,
        help = "Override the Tauri updater public key; defaults to FINI_UPDATE_PUBKEY or the build-time key"
    )]
    pubkey: Option<String>,
    #[arg(
        long,
        help = "Override the Tauri updater target; defaults to cli-<platform>-<arch>"
    )]
    target: Option<String>,
    #[arg(
        long,
        hide = true,
        help = "Override the executable path passed to the Tauri updater"
    )]
    executable_path: Option<std::path::PathBuf>,
}

#[derive(Subcommand)]
enum FocusCommand {
    Get,
    Set(FocusSetArgs),
}

#[derive(Args)]
struct FocusSetArgs {
    #[arg(long)]
    quest_id: String,
    #[arg(long)]
    trigger: Option<String>,
}

#[derive(Subcommand)]
enum QuestCommand {
    List(QuestListArgs),
    Get(IdArg),
    Create(QuestCreateArgs),
    Update(QuestUpdateArgs),
    Complete(IdArg),
    Abandon(IdArg),
    Delete(IdArg),
    History,
}

#[derive(Args)]
struct QuestListArgs {
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    space_id: Option<String>,
}

#[derive(Args)]
struct QuestCreateArgs {
    #[arg(long)]
    title: String,
    #[arg(long)]
    space_id: Option<String>,
    #[arg(long)]
    description: Option<String>,
    #[arg(long)]
    due: Option<String>,
    #[arg(long)]
    due_time: Option<String>,
    #[arg(long)]
    repeat_rule: Option<String>,
}

#[derive(Args)]
struct QuestUpdateArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    description: Option<String>,
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    space_id: Option<String>,
    #[arg(long)]
    pinned: Option<bool>,
    #[arg(long)]
    due: Option<String>,
    #[arg(long)]
    due_time: Option<String>,
    #[arg(long)]
    repeat_rule: Option<String>,
    #[arg(long)]
    order_rank: Option<f64>,
    #[arg(long, action = ArgAction::SetTrue)]
    set_focus: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    trigger_reminder_focus: bool,
}

#[derive(Subcommand)]
enum SpaceCommand {
    List,
    Create(SpaceCreateArgs),
    Update(SpaceUpdateArgs),
    Delete(IdArg),
}

#[derive(Args)]
struct SpaceCreateArgs {
    #[arg(long)]
    name: String,
}

#[derive(Args)]
struct SpaceUpdateArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    name: Option<String>,
}

#[derive(Subcommand)]
enum BackupCommand {
    Export(BackupExportArgs),
    Import(BackupImportArgs),
}

#[derive(Args)]
struct BackupExportArgs {
    #[arg(long)]
    path: String,
    #[arg(long = "space-id")]
    space_id: Vec<String>,
    #[arg(long, action = ArgAction::SetTrue)]
    all_spaces: bool,
}

#[derive(Args)]
struct BackupImportArgs {
    #[arg(long)]
    path: String,
    #[arg(long, action = ArgAction::SetTrue)]
    force: bool,
}

#[derive(Subcommand)]
enum ReminderCommand {
    List(QuestIdArg),
    Create(ReminderCreateArgs),
    Delete(IdArg),
}

#[derive(Args)]
struct ReminderCreateArgs {
    #[arg(long)]
    quest_id: String,
    #[arg(long = "type")]
    kind: String,
    #[arg(long)]
    mm_offset: Option<i64>,
    #[arg(long)]
    due_at_utc: Option<String>,
}

#[derive(Subcommand)]
enum DeviceCommand {
    Identity,
    AddMode {
        #[command(subcommand)]
        command: DeviceAddModeCommand,
    },
    Discovery,
    Presence,
    Pair {
        #[command(subcommand)]
        command: DevicePairCommand,
    },
    Paired {
        #[command(subcommand)]
        command: DevicePairedCommand,
    },
    Updates {
        #[command(subcommand)]
        command: DeviceUpdatesCommand,
    },
    Debug {
        #[command(subcommand)]
        command: DeviceDebugCommand,
    },
}

#[derive(Subcommand)]
enum DeviceAddModeCommand {
    Enter,
    Leave,
}

#[derive(Subcommand)]
enum DevicePairCommand {
    Send(DevicePairSendArgs),
    Incoming,
    OutgoingUpdates,
    OutgoingCompletions,
    Accept(RequestIdArg),
    Complete(RequestIdArg),
    Acknowledge(RequestIdArg),
}

#[derive(Args)]
struct DevicePairSendArgs {
    #[arg(long)]
    request_id: String,
    #[arg(long)]
    to_device_id: String,
    #[arg(long)]
    to_addr: String,
    #[arg(long)]
    to_ws_port: Option<u16>,
}

#[derive(Subcommand)]
enum DevicePairedCommand {
    List,
    Save(DevicePairedSaveArgs),
    UpdateLastSeen(DevicePairedLastSeenArgs),
    Unpair(PeerDeviceIdArg),
}

#[derive(Args)]
struct DevicePairedSaveArgs {
    #[arg(long)]
    peer_device_id: String,
    #[arg(long)]
    display_name: String,
}

#[derive(Args)]
struct DevicePairedLastSeenArgs {
    #[arg(long)]
    peer_device_id: String,
    #[arg(long)]
    last_seen_at: String,
}

#[derive(Subcommand)]
enum DeviceUpdatesCommand {
    ConsumeSpaceMapping,
}

#[derive(Subcommand)]
enum DeviceDebugCommand {
    Status,
}

#[derive(Subcommand)]
enum SyncCommand {
    Mappings {
        #[command(subcommand)]
        command: SyncMappingsCommand,
    },
    Tick {
        #[arg(long)]
        peer_device_id: Option<String>,
    },
    Status {
        #[arg(long)]
        peer_device_id: Option<String>,
    },
}

#[derive(Subcommand)]
enum SyncMappingsCommand {
    List(PeerDeviceIdArg),
    Update(SyncMappingsUpdateArgs),
    ApplyRemote(SyncMappingsApplyRemoteArgs),
    ResolveCustom(SyncResolveCustomArgs),
}

#[derive(Args)]
struct SyncMappingsUpdateArgs {
    #[arg(long)]
    peer_device_id: String,
    #[arg(long = "mapped-space-id")]
    mapped_space_id: Vec<String>,
}

#[derive(Args)]
struct SyncMappingsApplyRemoteArgs {
    #[arg(long)]
    peer_device_id: String,
    #[arg(long = "mapped-space-id")]
    mapped_space_id: Vec<String>,
}

#[derive(Args)]
struct SyncResolveCustomArgs {
    #[arg(long)]
    peer_device_id: String,
    #[arg(long = "space-id")]
    space_id: String,
    #[arg(long = "space-name")]
    space_name: Option<String>,
    #[arg(long = "mode")]
    mode: String,
    #[arg(long = "existing-local-space-id")]
    existing_local_space_id: Option<String>,
}

#[derive(Subcommand)]
enum SettingsCommand {
    ThemeGet,
    ThemeSet(SettingsThemeSetArgs),
    ThemeHint,
}

#[derive(Args)]
struct SettingsThemeSetArgs {
    #[arg(long)]
    mode: String,
}

#[derive(Args)]
struct IdArg {
    #[arg(long)]
    id: String,
}

#[derive(Args)]
struct QuestIdArg {
    #[arg(long)]
    quest_id: String,
}

#[derive(Args)]
struct RequestIdArg {
    #[arg(long)]
    request_id: String,
}

#[derive(Args)]
struct PeerDeviceIdArg {
    #[arg(long)]
    peer_device_id: String,
}

struct CliError {
    code: i32,
    message: String,
}

impl CliError {
    fn runtime(message: impl Into<String>) -> Self {
        Self {
            code: EXIT_RUNTIME,
            message: message.into(),
        }
    }

    fn from_string(message: String) -> Self {
        let lower = message.to_ascii_lowercase();
        let code = if lower.contains("not found") {
            EXIT_NOT_FOUND
        } else if lower.contains("invalid") || lower.contains("cannot") {
            EXIT_INVALID_STATE
        } else {
            EXIT_RUNTIME
        };
        Self { code, message }
    }
}

type CliResult<T> = Result<T, CliError>;

struct CliContext {
    db_path: std::path::PathBuf,
    device_state: DeviceConnectionState,
}

impl CliContext {
    fn new() -> CliResult<Self> {
        let db_path = db_default_path();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| CliError::runtime(format!("failed to create data dir: {err}")))?;
        }
        let app_data_dir = db_path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let device_state = DeviceConnectionState::try_from_db_path(&app_data_dir, db_path.clone())
            .map_err(CliError::runtime)?;
        Ok(Self {
            db_path,
            device_state,
        })
    }

    fn open_db(&self) -> CliResult<SqliteConnection> {
        open_cli_db_at_path(&self.db_path)
    }
}

fn open_cli_db_at_path(path: &std::path::Path) -> CliResult<SqliteConnection> {
    try_open_db_at_path(path).map_err(CliError::runtime)
}

pub fn run(args: Vec<String>) -> i32 {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => {
            let code = err.exit_code();
            let _ = err.print();
            return code;
        }
    };

    match execute(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{}", err.message);
            err.code
        }
    }
}

fn execute(cli: Cli) -> CliResult<i32> {
    if let Some(Command::Update(args)) = cli.command {
        let value = run_update(UpdateOptions {
            dry_run: args.dry_run,
            endpoint: args.endpoint,
            pubkey: args.pubkey,
            target: args.target,
            executable_path: args.executable_path,
        })
        .map_err(CliError::runtime)?;
        print_output(&value, cli.json).map_err(CliError::runtime)?;
        return Ok(EXIT_SUCCESS);
    }

    if should_run_auto_update() {
        maybe_auto_update();
    }

    let ctx = CliContext::new()?;
    let value = match cli.command {
        None => handle_focus(&ctx, FocusCommand::Get)?,
        Some(Command::Update(_)) => unreachable!("update is handled before DB initialization"),
        Some(Command::Focus { command }) => handle_focus(&ctx, command)?,
        Some(Command::Quest { command }) => handle_quest(&ctx, command)?,
        Some(Command::Space { command }) => handle_space(&ctx, command)?,
        Some(Command::Backup { command }) => handle_backup(&ctx, command)?,
        Some(Command::Reminder { command }) => handle_reminder(&ctx, command)?,
        Some(Command::Device { command }) => handle_device(&ctx, command)?,
        Some(Command::Sync { command }) => handle_sync(&ctx, command)?,
        Some(Command::Settings { command }) => handle_settings(&ctx, command)?,
    };
    print_output(&value, cli.json).map_err(CliError::runtime)?;
    Ok(EXIT_SUCCESS)
}

fn should_run_auto_update() -> bool {
    !cfg!(debug_assertions) && std::env::var_os("FINI_DISABLE_AUTO_UPDATE").is_none()
}

fn emit_quest_sync(
    ctx: &CliContext,
    conn: &mut diesel::sqlite::SqliteConnection,
    quest: &crate::models::Quest,
    op_type: &str,
) {
    let payload = if op_type == "delete" {
        None
    } else {
        serde_json::to_string(quest).ok()
    };
    let _ = emit_sync_event(
        conn,
        &ctx.device_state.identity.device_id,
        "quest",
        &quest.id,
        &quest.space_id,
        op_type,
        payload,
    );
}

fn emit_series_sync(
    ctx: &CliContext,
    conn: &mut diesel::sqlite::SqliteConnection,
    series: &crate::models::QuestSeries,
) {
    let payload = serde_json::to_string(series).ok();
    let _ = emit_sync_event(
        conn,
        &ctx.device_state.identity.device_id,
        "quest_series",
        &series.id,
        &series.space_id,
        "upsert",
        payload,
    );
}

fn handle_focus(ctx: &CliContext, command: FocusCommand) -> CliResult<Value> {
    let mut conn = ctx.open_db()?;
    match command {
        FocusCommand::Get => {
            let quest = resolve_active_quest(&mut conn)
                .map_err(|e| CliError::runtime(e.to_string()))?
                .map(|quest| record_focus_enter(&mut conn, &quest))
                .transpose()
                .map_err(|e| CliError::runtime(e.to_string()))?;
            quest
                .map(|quest| {
                    serde_json::to_value(quest).map_err(|e| CliError::runtime(e.to_string()))
                })
                .transpose()
                .map(|value| value.unwrap_or(Value::Null))
        }
        FocusCommand::Set(args) => {
            let quest = quests::table
                .find(&args.quest_id)
                .filter(quests::status.eq("active"))
                .select(crate::models::Quest::as_select())
                .first(&mut conn)
                .optional()
                .map_err(|e| CliError::runtime(e.to_string()))?
                .ok_or_else(|| {
                    CliError::from_string("cannot set Focus on non-active quest".to_string())
                })?;
            diesel::update(quests::table.find(&quest.id))
                .set(quests::updated_at.eq(utc_now()))
                .execute(&mut conn)
                .map_err(|e| CliError::runtime(e.to_string()))?;
            let trigger = if args.trigger.as_deref() == Some("reminder") {
                "reminder"
            } else {
                "manual"
            };
            append_focus_history(&mut conn, &quest.id, &quest.space_id, trigger)
                .map_err(CliError::from_string)?;
            let quest = record_focus_enter(&mut conn, &quest)
                .map_err(|e| CliError::runtime(e.to_string()))?;
            serde_json::to_value(quest).map_err(|e| CliError::runtime(e.to_string()))
        }
    }
}

fn handle_quest(ctx: &CliContext, command: QuestCommand) -> CliResult<Value> {
    let mut conn = ctx.open_db()?;
    match command {
        QuestCommand::List(args) => {
            let status = args.status.unwrap_or_else(|| "active".to_string());
            let quests: Vec<_> = QuestRepository::new(&mut conn)
                .list_for_ui()
                .map_err(CliError::from_string)?
                .into_iter()
                .filter(|quest| quest.status == status)
                .filter(|quest| {
                    args.space_id
                        .as_ref()
                        .map(|id| &quest.space_id == id)
                        .unwrap_or(true)
                })
                .collect();
            Ok(json!({ "quests": quests }))
        }
        QuestCommand::Get(args) => serde_json::to_value(
            QuestRepository::new(&mut conn)
                .get(&args.id)
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string())),
        QuestCommand::Create(args) => {
            let input = CreateQuestInput {
                space_id: args.space_id.unwrap_or_else(|| "1".to_string()),
                title: args.title,
                description: args.description,
                energy: "medium".to_string(),
                priority: 1,
                due: args.due,
                due_time: args.due_time,
                repeat_rule: args.repeat_rule,
                order_rank: None,
            };
            let created = QuestRepository::new(&mut conn)
                .create(input)
                .map_err(CliError::from_string)?;
            if let Some(series) = &created.series {
                emit_series_sync(ctx, &mut conn, series);
            }
            if created.quest.status == "active" && created.quest.due.is_some() {
                let _ = ReminderRepository::new(&mut conn).upsert_for_quest(&created.quest);
            }
            emit_quest_sync(ctx, &mut conn, &created.quest, "upsert");
            serde_json::to_value(created.quest).map_err(|e| CliError::runtime(e.to_string()))
        }
        QuestCommand::Update(args) => update_quest_from_cli(ctx, &mut conn, args),
        QuestCommand::Complete(args) => update_quest_status(ctx, &mut conn, args.id, "completed"),
        QuestCommand::Abandon(args) => update_quest_status(ctx, &mut conn, args.id, "abandoned"),
        QuestCommand::Delete(args) => {
            let quest = QuestRepository::new(&mut conn)
                .delete(&args.id)
                .map_err(CliError::from_string)?;
            let _ = ReminderRepository::new(&mut conn).delete_for_quest(&quest.id);
            emit_quest_sync(ctx, &mut conn, &quest, "delete");
            Ok(json!({ "deleted": true, "id": args.id }))
        }
        QuestCommand::History => {
            let mut quests: Vec<_> = QuestRepository::new(&mut conn)
                .list_for_ui()
                .map_err(CliError::from_string)?
                .into_iter()
                .filter(|quest| quest.status != "active")
                .collect();
            quests.sort_by(|a, b| {
                b.updated_at
                    .cmp(&a.updated_at)
                    .then_with(|| a.id.cmp(&b.id))
            });
            Ok(json!({ "quests": quests }))
        }
    }
}

fn update_quest_from_cli(
    ctx: &CliContext,
    conn: &mut diesel::sqlite::SqliteConnection,
    args: QuestUpdateArgs,
) -> CliResult<Value> {
    let input = UpdateQuestInput {
        space_id: args.space_id,
        title: args.title,
        description: args.description,
        status: args.status.clone(),
        energy: None,
        priority: None,
        pinned: args.pinned,
        due: args.due,
        due_time: args.due_time,
        repeat_rule: args.repeat_rule,
        order_rank: args.order_rank,
    };
    let result = QuestRepository::new(conn)
        .update(&args.id, input)
        .map_err(CliError::from_string)?;
    if args.trigger_reminder_focus {
        append_focus_history(conn, &result.quest.id, &result.quest.space_id, "reminder")
            .map_err(CliError::from_string)?;
    } else if args.set_focus
        || args.status.as_deref() == Some("active")
        || result.restore_should_focus
    {
        append_focus_history(conn, &result.quest.id, &result.quest.space_id, "manual")
            .map_err(CliError::from_string)?;
    }
    sync_reminder_rows(conn, &result.quest);
    if let Some(ref occurrence) = result.next_occurrence {
        sync_reminder_rows(conn, occurrence);
    }
    emit_quest_sync(ctx, conn, &result.quest, "upsert");
    serde_json::to_value(result.quest).map_err(|e| CliError::runtime(e.to_string()))
}

fn update_quest_status(
    ctx: &CliContext,
    conn: &mut diesel::sqlite::SqliteConnection,
    id: String,
    status: &str,
) -> CliResult<Value> {
    update_quest_from_cli(
        ctx,
        conn,
        QuestUpdateArgs {
            id,
            title: None,
            description: None,
            status: Some(status.to_string()),
            space_id: None,
            pinned: None,
            due: None,
            due_time: None,
            repeat_rule: None,
            order_rank: None,
            set_focus: false,
            trigger_reminder_focus: false,
        },
    )
}

fn sync_reminder_rows(conn: &mut diesel::sqlite::SqliteConnection, quest: &crate::models::Quest) {
    if quest.status == "active" && quest.due.is_some() {
        let _ = ReminderRepository::new(conn).upsert_for_quest(quest);
    } else {
        let _ = ReminderRepository::new(conn).delete_for_quest(&quest.id);
    }
}

fn handle_space(ctx: &CliContext, command: SpaceCommand) -> CliResult<Value> {
    let mut conn = ctx.open_db()?;
    match command {
        SpaceCommand::List => Ok(json!({
            "spaces": SpaceRepository::new(&mut conn).list().map_err(CliError::from_string)?
        })),
        SpaceCommand::Create(args) => {
            let item_order = SpaceRepository::new(&mut conn)
                .list()
                .map(|spaces| spaces.len() as i64)
                .unwrap_or(0);
            serde_json::to_value(
                SpaceRepository::new(&mut conn)
                    .create(CreateSpaceInput {
                        name: args.name,
                        item_order,
                    })
                    .map_err(CliError::from_string)?,
            )
            .map_err(|e| CliError::runtime(e.to_string()))
        }
        SpaceCommand::Update(args) => serde_json::to_value(
            SpaceRepository::new(&mut conn)
                .update(
                    &args.id,
                    UpdateSpaceInput {
                        name: args.name,
                        item_order: None,
                    },
                )
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string())),
        SpaceCommand::Delete(args) => {
            SpaceRepository::new(&mut conn)
                .delete(&args.id)
                .map_err(CliError::from_string)?;
            Ok(json!({ "deleted": true, "id": args.id }))
        }
    }
}

fn handle_backup(ctx: &CliContext, command: BackupCommand) -> CliResult<Value> {
    let mut conn = ctx.open_db()?;
    let value = match command {
        BackupCommand::Export(args) => {
            if args.all_spaces && !args.space_id.is_empty() {
                return Err(CliError::from_string(
                    "cannot combine --all-spaces with --space-id".to_string(),
                ));
            }
            let space_ids = if args.all_spaces {
                crate::schema::spaces::table
                    .select(crate::schema::spaces::id)
                    .load::<String>(&mut conn)
                    .map_err(|e| CliError::runtime(e.to_string()))?
            } else if args.space_id.is_empty() {
                return Err(CliError::from_string(
                    "backup export requires --space-id or --all-spaces".to_string(),
                ));
            } else {
                args.space_id
            };
            serde_json::to_value(
                backup::export_backup(&mut conn, std::path::Path::new(&args.path), &space_ids)
                    .map_err(CliError::from_string)?,
            )
            .map_err(|e| CliError::runtime(e.to_string()))?
        }
        BackupCommand::Import(args) => serde_json::to_value(
            backup::import_cli(&mut conn, std::path::Path::new(&args.path), args.force)
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string()))?,
    };
    Ok(value)
}

fn handle_reminder(ctx: &CliContext, command: ReminderCommand) -> CliResult<Value> {
    let mut conn = ctx.open_db()?;
    match command {
        ReminderCommand::List(args) => serde_json::to_value(
            ReminderRepository::new(&mut conn)
                .list_for_quest(&args.quest_id)
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string())),
        ReminderCommand::Create(args) => serde_json::to_value(
            ReminderRepository::new(&mut conn)
                .create(CreateReminderInput {
                    quest_id: args.quest_id,
                    kind: args.kind,
                    mm_offset: args.mm_offset,
                    due_at_utc: args.due_at_utc,
                })
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string())),
        ReminderCommand::Delete(args) => {
            ReminderRepository::new(&mut conn)
                .delete(&args.id)
                .map_err(CliError::from_string)?;
            Ok(json!({ "deleted": true, "id": args.id }))
        }
    }
}

fn handle_device(ctx: &CliContext, command: DeviceCommand) -> CliResult<Value> {
    let mut conn = ctx.open_db()?;
    let value = match command {
        DeviceCommand::Identity => serde_json::to_value(
            device_connection_get_identity_impl(&ctx.device_state)
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string()))?,
        DeviceCommand::AddMode { command } => {
            match command {
                DeviceAddModeCommand::Enter => {
                    device_connection_enter_add_mode_impl(&ctx.device_state)
                }
                DeviceAddModeCommand::Leave => {
                    device_connection_leave_add_mode_impl(&ctx.device_state)
                }
            }
            .map_err(CliError::from_string)?;
            json!({ "ok": true })
        }
        DeviceCommand::Discovery => serde_json::to_value(
            device_connection_discovery_snapshot_impl(&ctx.device_state)
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string()))?,
        DeviceCommand::Presence => serde_json::to_value(
            device_connection_presence_snapshot_impl(&ctx.device_state)
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string()))?,
        DeviceCommand::Pair { command } => handle_device_pair(ctx, command)?,
        DeviceCommand::Paired { command } => match command {
            DevicePairedCommand::List => serde_json::to_value(
                device_connection_get_paired_devices_impl(&mut conn)
                    .map_err(CliError::from_string)?,
            )
            .map_err(|e| CliError::runtime(e.to_string()))?,
            DevicePairedCommand::Save(args) => serde_json::to_value(
                device_connection_save_paired_device_impl(
                    &mut conn,
                    args.peer_device_id,
                    args.display_name,
                )
                .map_err(CliError::from_string)?,
            )
            .map_err(|e| CliError::runtime(e.to_string()))?,
            DevicePairedCommand::UpdateLastSeen(args) => {
                device_connection_update_last_seen_impl(
                    &mut conn,
                    args.peer_device_id,
                    args.last_seen_at,
                )
                .map_err(CliError::from_string)?;
                json!({ "ok": true })
            }
            DevicePairedCommand::Unpair(args) => {
                device_connection_unpair_impl(&mut conn, args.peer_device_id)
                    .map_err(CliError::from_string)?;
                json!({ "ok": true })
            }
        },
        DeviceCommand::Updates { command } => match command {
            DeviceUpdatesCommand::ConsumeSpaceMapping => serde_json::to_value(
                device_connection_consume_space_mapping_updates_impl(&ctx.device_state)
                    .map_err(CliError::from_string)?,
            )
            .map_err(|e| CliError::runtime(e.to_string()))?,
        },
        DeviceCommand::Debug { command } => match command {
            DeviceDebugCommand::Status => serde_json::to_value(
                device_connection_debug_status_impl(&ctx.device_state)
                    .map_err(CliError::from_string)?,
            )
            .map_err(|e| CliError::runtime(e.to_string()))?,
        },
    };
    Ok(value)
}

fn handle_device_pair(ctx: &CliContext, command: DevicePairCommand) -> CliResult<Value> {
    let value = match command {
        DevicePairCommand::Send(args) => {
            device_connection_send_pair_request_impl(
                &ctx.device_state,
                DevicePairRequestInput {
                    request_id: args.request_id,
                    to_device_id: args.to_device_id,
                    to_addr: args.to_addr,
                    to_ws_port: args.to_ws_port,
                },
            )
            .map_err(CliError::from_string)?;
            json!({ "ok": true })
        }
        DevicePairCommand::Incoming => serde_json::to_value(
            device_connection_pair_incoming_requests_impl(&ctx.device_state)
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string()))?,
        DevicePairCommand::OutgoingUpdates => serde_json::to_value(
            device_connection_pair_outgoing_updates_impl(&ctx.device_state)
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string()))?,
        DevicePairCommand::OutgoingCompletions => serde_json::to_value(
            device_connection_pair_outgoing_completions_impl(&ctx.device_state)
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string()))?,
        DevicePairCommand::Accept(args) => serde_json::to_value(
            device_connection_pair_accept_request_impl(
                &ctx.device_state,
                DevicePairRequestAckInput {
                    request_id: args.request_id,
                },
            )
            .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string()))?,
        DevicePairCommand::Complete(args) => {
            device_connection_pair_complete_request_impl(
                &ctx.device_state,
                DevicePairRequestAckInput {
                    request_id: args.request_id,
                },
            )
            .map_err(CliError::from_string)?;
            json!({ "ok": true })
        }
        DevicePairCommand::Acknowledge(args) => {
            device_connection_pair_acknowledge_request_impl(
                &ctx.device_state,
                DevicePairRequestAckInput {
                    request_id: args.request_id,
                },
            )
            .map_err(CliError::from_string)?;
            json!({ "ok": true })
        }
    };
    Ok(value)
}

fn handle_sync(ctx: &CliContext, command: SyncCommand) -> CliResult<Value> {
    let mut conn = ctx.open_db()?;
    let value = match command {
        SyncCommand::Mappings { command } => match command {
            SyncMappingsCommand::List(args) => serde_json::to_value(
                space_sync_list_mappings_impl(&mut conn, args.peer_device_id)
                    .map_err(CliError::from_string)?,
            )
            .map_err(|e| CliError::runtime(e.to_string()))?,
            SyncMappingsCommand::Update(args) => serde_json::to_value(
                space_sync_update_mappings_impl(
                    &mut conn,
                    &ctx.device_state,
                    args.peer_device_id,
                    args.mapped_space_id,
                )
                .map_err(CliError::from_string)?,
            )
            .map_err(|e| CliError::runtime(e.to_string()))?,
            SyncMappingsCommand::ApplyRemote(args) => serde_json::to_value(
                space_sync_apply_remote_mappings_impl(
                    &mut conn,
                    &ctx.device_state,
                    args.peer_device_id,
                    args.mapped_space_id,
                    Vec::new(),
                )
                .map_err(CliError::from_string)?,
            )
            .map_err(|e| CliError::runtime(e.to_string()))?,
            SyncMappingsCommand::ResolveCustom(args) => serde_json::to_value(
                space_sync_resolve_custom_space_mapping_impl(
                    &mut conn,
                    &ctx.device_state,
                    args.peer_device_id,
                    args.space_id,
                    args.space_name,
                    parse_space_resolution_mode(&args.mode)?,
                    args.existing_local_space_id,
                )
                .map_err(CliError::from_string)?,
            )
            .map_err(|e| CliError::runtime(e.to_string()))?,
        },
        SyncCommand::Tick { peer_device_id } => {
            let result = space_sync_tick_impl(&mut conn, &ctx.device_state)
                .map_err(CliError::from_string)?;
            if let Some(peer_id) = peer_device_id {
                let peers: Vec<Value> = result
                    .peers
                    .iter()
                    .filter(|peer| peer.peer_device_id == peer_id)
                    .map(|peer| serde_json::to_value(peer).unwrap_or(Value::Null))
                    .collect();
                json!({
                    "sent_events": result.sent_events,
                    "applied_events": result.applied_events,
                    "received_acks": result.received_acks,
                    "peers": peers,
                    "ticked_at": result.ticked_at,
                })
            } else {
                serde_json::to_value(result).map_err(|e| CliError::runtime(e.to_string()))?
            }
        }
        SyncCommand::Status { peer_device_id } => serde_json::to_value(
            space_sync_status_impl(&mut conn, peer_device_id).map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string()))?,
    };
    Ok(value)
}

fn parse_space_resolution_mode(value: &str) -> CliResult<SpaceResolutionMode> {
    match value {
        "create_new" => Ok(SpaceResolutionMode::CreateNew),
        "use_existing" => Ok(SpaceResolutionMode::UseExisting),
        _ => Err(CliError::from_string(
            "mode must be create_new or use_existing".to_string(),
        )),
    }
}

fn handle_settings(ctx: &CliContext, command: SettingsCommand) -> CliResult<Value> {
    let mut conn = ctx.open_db()?;
    let value = match command {
        SettingsCommand::ThemeGet => {
            let mode = settings::theme_mode(&mut conn).map_err(CliError::from_string)?;
            json!({ "mode": mode.as_str() })
        }
        SettingsCommand::ThemeSet(args) => {
            let mode = ThemeMode::parse(&args.mode).ok_or_else(|| {
                CliError::from_string("mode must be system, light, or dark".to_string())
            })?;
            let mode = settings::set_theme_mode(&mut conn, mode).map_err(CliError::from_string)?;
            json!({ "mode": mode.as_str() })
        }
        SettingsCommand::ThemeHint => json!({ "theme": settings::theme_hint(&mut conn) }),
    };
    Ok(value)
}

fn print_output(value: &Value, json: bool) -> Result<(), String> {
    if json {
        let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
        println!("{text}");
        return Ok(());
    }

    println!("{}", format_human_output(value));
    Ok(())
}

fn format_human_output(value: &Value) -> String {
    format_human_output_lines(value).join("\n")
}

fn format_human_output_lines(value: &Value) -> Vec<String> {
    match value {
        Value::Null => vec!["No active Focus quest.".to_string()],
        Value::Object(map) if map.contains_key("deleted") => {
            let id = map.get("id").and_then(Value::as_str).unwrap_or("unknown");
            vec![format!("Deleted: {id}")]
        }
        Value::Object(map) if map.get("ok").and_then(Value::as_bool) == Some(true) => {
            vec!["OK.".to_string()]
        }
        Value::Object(map) if map.contains_key("quests") => {
            let items = map
                .get("quests")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if items.is_empty() {
                vec!["No quests.".to_string()]
            } else {
                items
                    .iter()
                    .map(|item| format_quest_summary(item, "-"))
                    .collect()
            }
        }
        Value::Object(map) if map.contains_key("spaces") => {
            let items = map
                .get("spaces")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if items.is_empty() {
                vec!["No spaces.".to_string()]
            } else {
                items
                    .iter()
                    .map(|item| format_space_summary(item, "-"))
                    .collect()
            }
        }
        Value::Object(map) if is_quest_object(map) => vec![format_quest_summary(value, "Quest:")],
        Value::Object(map) if is_space_object(map) => vec![format_space_summary(value, "Space:")],
        Value::Object(map) if map.contains_key("mode") => {
            let mode = map
                .get("mode")
                .map(format_scalar)
                .unwrap_or_else(|| "unknown".to_string());
            vec![format!("Theme mode: {mode}")]
        }
        Value::Object(map) if map.contains_key("theme") => {
            let theme = map
                .get("theme")
                .map(format_scalar)
                .unwrap_or_else(|| "unknown".to_string());
            vec![format!("Theme hint: {theme}")]
        }
        Value::Array(items) => {
            if items.is_empty() {
                vec!["No results.".to_string()]
            } else {
                items.iter().map(format_generic_summary).collect()
            }
        }
        _ => vec![format_generic_summary(value)],
    }
}

fn is_quest_object(map: &serde_json::Map<String, Value>) -> bool {
    map.contains_key("id") && map.contains_key("title") && map.contains_key("status")
}

fn is_space_object(map: &serde_json::Map<String, Value>) -> bool {
    map.contains_key("id") && map.contains_key("name") && map.contains_key("item_order")
}

fn format_quest_summary(value: &Value, prefix: &str) -> String {
    let id = value.get("id").and_then(Value::as_str).unwrap_or("?");
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("(untitled)");
    let status = value.get("status").and_then(Value::as_str).unwrap_or("?");
    format!("{prefix} [{status}] {title} ({id})")
}

fn format_space_summary(value: &Value, prefix: &str) -> String {
    let id = value.get("id").and_then(Value::as_str).unwrap_or("?");
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("(unnamed)");
    format!("{prefix} {name} ({id})")
}

fn format_generic_summary(value: &Value) -> String {
    match value {
        Value::Null => "No result.".to_string(),
        Value::Bool(_) | Value::Number(_) | Value::String(_) => format_scalar(value),
        Value::Array(items) => format_count("result", items.len()),
        Value::Object(map) => {
            let fields: Vec<_> = map
                .iter()
                .filter_map(|(key, value)| match value {
                    Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                        Some(format!("{key}={}", format_scalar(value)))
                    }
                    Value::Array(items) => {
                        Some(format!("{key}={}", format_count("item", items.len())))
                    }
                    Value::Object(nested) => Some(format!("{key}={} fields", nested.len())),
                })
                .take(4)
                .collect();

            if fields.is_empty() {
                format!("Result: {} fields.", map.len())
            } else {
                format!("Result: {}.", fields.join(", "))
            }
        }
    }
}

fn format_count(noun: &str, count: usize) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn format_scalar(value: &Value) -> String {
    match value {
        Value::Null => "none".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) if value.is_empty() => "(empty)".to_string(),
        Value::String(value) => value.clone(),
        Value::Array(items) => format_count("item", items.len()),
        Value::Object(map) => format!("{} fields", map.len()),
    }
}

#[cfg(test)]
mod tests {
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
        diesel::sql_query(
            "INSERT INTO __diesel_schema_migrations (version) VALUES ('99999999999999')",
        )
        .execute(&mut conn)
        .expect("seed unknown future migration version");
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
}
