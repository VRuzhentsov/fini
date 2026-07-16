use clap::{ArgAction, ArgGroup, Args, Parser, Subcommand};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde_json::{json, Value};

use crate::models::{
    CreateQuestInput, CreateReminderInput, CreateSpaceInput, QuestUpdatePatch, UpdateQuestInput,
    UpdateSpaceInput,
};

use crate::services::backup;
use crate::services::cli_update::{maybe_auto_update, run_update, UpdateOptions};
use crate::services::db::{db_default_path, try_open_db_at_path};
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
    append_focus_history, record_focus_enter, resolve_active_quest, QuestService,
};
use crate::services::reminder::ReminderService;
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
    #[arg(long, global = true, help = "Print structured JSON output")]
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
    #[command(about = "Export selected spaces to a backup archive")]
    Export(BackupExportArgs),
    #[command(about = "Inspect a backup archive without changing local data")]
    Import(BackupInspectArgs),
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
    #[arg(long, help = "Text, or literal null to clear; omit to preserve")]
    description: Option<String>,
    #[arg(long, help = "Text, or literal null to clear; omit to preserve")]
    due: Option<String>,
    #[arg(long, help = "Text, or literal null to clear; omit to preserve")]
    due_time: Option<String>,
    #[arg(long, help = "Text, or literal null to clear; omit to preserve")]
    repeat_rule: Option<String>,
}

#[derive(Args)]
struct QuestUpdateArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    title: Option<String>,
    #[arg(long, help = "Text, or literal null to clear; omit to preserve")]
    description: Option<String>,
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    space_id: Option<String>,
    #[arg(long)]
    pinned: Option<bool>,
    #[arg(long, help = "Text, or literal null to clear; omit to preserve")]
    due: Option<String>,
    #[arg(long, help = "Text, or literal null to clear; omit to preserve")]
    due_time: Option<String>,
    #[arg(long, help = "Text, or literal null to clear; omit to preserve")]
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
    #[arg(long, help = "Destination backup archive path")]
    path: String,
    #[arg(
        long = "space-id",
        help = "Space ID to export; repeat for multiple spaces"
    )]
    space_id: Vec<String>,
    #[arg(long, action = ArgAction::SetTrue, help = "Export every space")]
    all_spaces: bool,
}

#[derive(Args)]
struct BackupImportArgs {
    #[arg(long)]
    path: String,
    #[arg(long, action = ArgAction::SetTrue)]
    force: bool,
}

#[derive(Args)]
#[command(group(
    ArgGroup::new("preflight_mode")
        .args(["verify", "dry_run"])
        .multiple(false)
))]
struct BackupInspectArgs {
    #[arg(long, help = "Backup archive path to inspect or verify")]
    path: String,
    #[arg(
        long,
        action = ArgAction::SetTrue,
        conflicts_with_all = ["verify", "dry_run"],
        help = "Validate the archive and print its contents as JSON"
    )]
    inspect: bool,
    #[arg(
        long,
        action = ArgAction::SetTrue,
        conflicts_with_all = ["inspect", "dry_run"],
        help = "Compare the archive with the local database without changing either"
    )]
    verify: bool,
    #[arg(
        long,
        action = ArgAction::SetTrue,
        conflicts_with_all = ["inspect", "verify"],
        help = "Create a read-only archive import plan without applying or recovering data"
    )]
    dry_run: bool,
    #[arg(
        long = "map-space",
        requires = "preflight_mode",
        help = "Backup space mapping BACKUP_ID=create_new or BACKUP_ID=use_existing:LOCAL_ID; repeat for multiple spaces"
    )]
    map_space: Vec<String>,
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

    if let Some(Command::Import(args)) = &cli.command {
        let value = handle_import(args)?;
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
        Some(Command::Export(args)) => handle_export(&ctx, args, "export")?,
        Some(Command::Import(_)) => {
            unreachable!("import inspection is handled before DB initialization")
        }
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
            let trigger = if args.trigger.as_deref() == Some("reminder") {
                "reminder"
            } else {
                "manual"
            };
            let quest = QuestService::new(&mut conn)
                .set_focus(&args.quest_id, trigger)
                .map_err(CliError::from_string)?;
            serde_json::to_value(quest).map_err(|e| CliError::runtime(e.to_string()))
        }
    }
}

fn handle_quest(ctx: &CliContext, command: QuestCommand) -> CliResult<Value> {
    let mut conn = ctx.open_db()?;
    match command {
        QuestCommand::List(args) => {
            let status = args.status.unwrap_or_else(|| "active".to_string());
            let quests: Vec<_> = QuestService::new(&mut conn)
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
            QuestService::new(&mut conn)
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
            let created = QuestService::new(&mut conn)
                .create(input)
                .map_err(CliError::from_string)?;
            if let Some(series) = &created.series {
                emit_series_sync(ctx, &mut conn, series);
            }
            if created.quest.status == "active" && created.quest.due.is_some() {
                let _ = ReminderService::new(&mut conn).upsert_for_quest(&created.quest);
            }
            emit_quest_sync(ctx, &mut conn, &created.quest, "upsert");
            serde_json::to_value(created.quest).map_err(|e| CliError::runtime(e.to_string()))
        }
        QuestCommand::Update(args) => update_quest_from_cli(ctx, &mut conn, args),
        QuestCommand::Complete(args) => update_quest_status(ctx, &mut conn, args.id, "completed"),
        QuestCommand::Abandon(args) => update_quest_status(ctx, &mut conn, args.id, "abandoned"),
        QuestCommand::Delete(args) => {
            let quest = QuestService::new(&mut conn)
                .delete(&args.id)
                .map_err(CliError::from_string)?;
            let _ = ReminderService::new(&mut conn).delete_for_quest(&quest.id);
            emit_quest_sync(ctx, &mut conn, &quest, "delete");
            Ok(json!({ "deleted": true, "id": args.id }))
        }
        QuestCommand::History => {
            let mut quests: Vec<_> = QuestService::new(&mut conn)
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

fn parse_nullable_quest_patch(value: Option<String>) -> crate::models::QuestFieldPatch<String> {
    match value {
        None => crate::models::QuestFieldPatch::Unchanged,
        Some(value) if value == "null" => crate::models::QuestFieldPatch::Clear,
        Some(value) => crate::models::QuestFieldPatch::Set(value),
    }
}

fn update_quest_from_cli(
    ctx: &CliContext,
    conn: &mut diesel::sqlite::SqliteConnection,
    args: QuestUpdateArgs,
) -> CliResult<Value> {
    let description = parse_nullable_quest_patch(args.description);
    let due = parse_nullable_quest_patch(args.due);
    let due_time = parse_nullable_quest_patch(args.due_time);
    let repeat_rule = parse_nullable_quest_patch(args.repeat_rule);
    let input = UpdateQuestInput {
        space_id: args.space_id,
        title: args.title,
        description: None,
        status: args.status.clone(),
        energy: None,
        priority: None,
        pinned: args.pinned,
        due: None,
        due_time: None,
        repeat_rule: None,
        order_rank: args.order_rank,
    };
    let patch = QuestUpdatePatch {
        input,
        description,
        due,
        due_time,
        repeat_rule,
    };
    let result = QuestService::new(conn)
        .update_patch(&args.id, patch)
        .map_err(CliError::from_string)?;
    let quest = result.quest;
    if args.trigger_reminder_focus {
        append_focus_history(conn, &quest.id, &quest.space_id, "reminder")
            .map_err(CliError::from_string)?;
    } else if args.set_focus
        || args.status.as_deref() == Some("active")
        || result.restore_should_focus
    {
        append_focus_history(conn, &quest.id, &quest.space_id, "manual")
            .map_err(CliError::from_string)?;
    }
    sync_reminder_rows(conn, &quest);
    if let Some(ref occurrence) = result.next_occurrence {
        sync_reminder_rows(conn, occurrence);
    }
    emit_quest_sync(ctx, conn, &quest, "upsert");
    serde_json::to_value(quest).map_err(|e| CliError::runtime(e.to_string()))
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
    let _ = ReminderService::new(conn).reconcile(quest);
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
    match command {
        BackupCommand::Export(args) => handle_export(ctx, args, "backup export"),
        BackupCommand::Import(args) => serde_json::to_value(
            backup::import_cli(
                &mut ctx.open_db()?,
                std::path::Path::new(&args.path),
                args.force,
            )
            .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string())),
    }
}

fn handle_export(ctx: &CliContext, args: BackupExportArgs, command_name: &str) -> CliResult<Value> {
    if args.all_spaces && !args.space_id.is_empty() {
        return Err(CliError::from_string(
            "cannot combine --all-spaces with --space-id".to_string(),
        ));
    }

    let mut conn = ctx.open_db()?;
    let space_ids = if args.all_spaces {
        crate::schema::spaces::table
            .select(crate::schema::spaces::id)
            .load::<String>(&mut conn)
            .map_err(|e| CliError::runtime(e.to_string()))?
    } else if args.space_id.is_empty() {
        return Err(CliError::from_string(format!(
            "{command_name} requires --space-id or --all-spaces"
        )));
    } else {
        args.space_id
    };

    serde_json::to_value(
        backup::export_backup(&mut conn, std::path::Path::new(&args.path), &space_ids)
            .map_err(CliError::from_string)?,
    )
    .map_err(|e| CliError::runtime(e.to_string()))
}

fn handle_import(args: &BackupInspectArgs) -> CliResult<Value> {
    if !args.map_space.is_empty() && !args.verify && !args.dry_run {
        return Err(CliError::from_string(
            "--map-space requires --verify or --dry-run".to_string(),
        ));
    }

    if args.inspect {
        return serde_json::to_value(
            backup::inspect_backup(std::path::Path::new(&args.path))
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string()));
    }

    if args.verify || args.dry_run {
        let db_path = db_default_path();
        if !db_path.is_file() {
            return Err(CliError::runtime(format!(
                "local database does not exist: {}",
                db_path.display()
            )));
        }
        let mut conn = SqliteConnection::establish(
            db_path
                .to_str()
                .ok_or_else(|| CliError::runtime("database path is not UTF-8"))?,
        )
        .map_err(|err| CliError::runtime(format!("failed to open local database: {err}")))?;
        conn.batch_execute("PRAGMA query_only = ON;")
            .map_err(|err| {
                CliError::runtime(format!("failed to make local database read-only: {err}"))
            })?;
        let mappings = parse_backup_space_mappings(&args.map_space)?;
        let preflight =
            backup::preflight_import(&mut conn, std::path::Path::new(&args.path), &mappings)
                .map_err(CliError::from_string)?;
        let output = if args.dry_run {
            serde_json::to_value(backup::BackupImportDryRunPlan::from_preflight(preflight))
        } else {
            serde_json::to_value(preflight)
        };
        return output.map_err(|e| CliError::runtime(e.to_string()));
    }

    Err(CliError::from_string(
        "import requires --inspect, --verify, or --dry-run; mutation modes are not available yet"
            .to_string(),
    ))
}

fn parse_backup_space_mappings(
    values: &[String],
) -> CliResult<Vec<backup::BackupSpaceMappingInput>> {
    let mut backup_space_ids = std::collections::HashSet::new();
    let mut mappings = Vec::with_capacity(values.len());

    for value in values {
        let (backup_space_id, mapping) = value.split_once('=').ok_or_else(|| {
            CliError::from_string(
                "--map-space must be BACKUP_ID=create_new or BACKUP_ID=use_existing:LOCAL_ID"
                    .to_string(),
            )
        })?;
        if backup_space_id.is_empty() {
            return Err(CliError::from_string(
                "--map-space backup space ID must not be empty".to_string(),
            ));
        }
        if !backup_space_ids.insert(backup_space_id) {
            return Err(CliError::from_string(format!(
                "--map-space backup space ID is duplicated: {backup_space_id}"
            )));
        }
        if mapping == "create_new" {
            mappings.push(backup::BackupSpaceMappingInput {
                backup_space_id: backup_space_id.to_string(),
                mode: mapping.to_string(),
                local_space_id: None,
            });
            continue;
        }
        let local_space_id = mapping.strip_prefix("use_existing:").ok_or_else(|| {
            CliError::from_string(
                "--map-space must be BACKUP_ID=create_new or BACKUP_ID=use_existing:LOCAL_ID"
                    .to_string(),
            )
        })?;
        if local_space_id.is_empty() {
            return Err(CliError::from_string(
                "--map-space existing local space ID must not be empty".to_string(),
            ));
        }
        mappings.push(backup::BackupSpaceMappingInput {
            backup_space_id: backup_space_id.to_string(),
            mode: "use_existing".to_string(),
            local_space_id: Some(local_space_id.to_string()),
        });
    }

    Ok(mappings)
}

fn handle_reminder(ctx: &CliContext, command: ReminderCommand) -> CliResult<Value> {
    let mut conn = ctx.open_db()?;
    match command {
        ReminderCommand::List(args) => serde_json::to_value(
            ReminderService::new(&mut conn)
                .list_for_quest(&args.quest_id)
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string())),
        ReminderCommand::Create(args) => serde_json::to_value(
            ReminderService::new(&mut conn)
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
            ReminderService::new(&mut conn)
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

    match value {
        Value::Null => println!("No active Focus quest."),
        Value::Object(map) if map.contains_key("deleted") => {
            let id = map.get("id").and_then(Value::as_str).unwrap_or("unknown");
            println!("Deleted: {id}");
        }
        Value::Object(map) if map.contains_key("quests") => {
            let items = map
                .get("quests")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if items.is_empty() {
                println!("No quests.");
            } else {
                for item in items {
                    let id = item.get("id").and_then(Value::as_str).unwrap_or("?");
                    let title = item
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("(untitled)");
                    let status = item.get("status").and_then(Value::as_str).unwrap_or("?");
                    println!("- [{status}] {title} ({id})");
                }
            }
        }
        Value::Object(map) if map.contains_key("spaces") => {
            let items = map
                .get("spaces")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if items.is_empty() {
                println!("No spaces.");
            } else {
                for item in items {
                    let id = item.get("id").and_then(Value::as_str).unwrap_or("?");
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("(unnamed)");
                    println!("- {name} ({id})");
                }
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                println!("No results.");
            } else {
                for item in items {
                    let text = serde_json::to_string_pretty(item).map_err(|e| e.to_string())?;
                    println!("{text}");
                }
            }
        }
        _ => {
            let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
            println!("{text}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::db::temp_db_path;
    use std::path::Path;

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
        let target_after =
            std::fs::read(&target_db_path).expect("read target database after inspect");
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
        let target_before =
            std::fs::read(&target_db_path).expect("read target database before verify");

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
        let target_after =
            std::fs::read(&target_db_path).expect("read target database after verify");
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
        let path_only = Cli::try_parse_from(["fini", "import", "--path", "backup.zip"]).expect(
            "path-only import parses so execution can explain the unavailable mutation mode",
        );
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
