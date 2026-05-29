use clap::{ArgAction, Args, Parser, Subcommand};
use serde_json::{json, Value};

use crate::services::backup;
use crate::services::db::{db_default_path, open_db_at_path};
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
use crate::services::mcp::{FiniServer, UpdateQuestParams};
use crate::services::settings::{self, ThemeMode};
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
    Mcp,
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
    server: FiniServer,
    runtime: tokio::runtime::Runtime,
}

impl CliContext {
    fn new() -> CliResult<Self> {
        let db_path = db_default_path();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| CliError::runtime(format!("failed to create data dir: {err}")))?;
        }

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| CliError::runtime(format!("failed to create tokio runtime: {err}")))?;

        let app_data_dir = db_path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let device_state = DeviceConnectionState::new_with_db_path(&app_data_dir, db_path.clone());
        let origin_device_id = Some(device_state.identity.device_id.clone());

        Ok(Self {
            db_path: db_path.clone(),
            device_state,
            server: FiniServer::new_with_origin(&db_path, origin_device_id),
            runtime,
        })
    }
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
    if let Some(Command::Mcp) = cli.command {
        crate::run_mcp();
        return Ok(EXIT_SUCCESS);
    }

    let ctx = CliContext::new()?;

    let value = match cli.command {
        None => ctx
            .runtime
            .block_on(ctx.server.cli_get_active_focus())
            .map_err(CliError::from_string)?,
        Some(Command::Focus { command }) => handle_focus(&ctx, command)?,
        Some(Command::Quest { command }) => handle_quest(&ctx, command)?,
        Some(Command::Space { command }) => handle_space(&ctx, command)?,
        Some(Command::Backup { command }) => handle_backup(&ctx, command)?,
        Some(Command::Reminder { command }) => handle_reminder(&ctx, command)?,
        Some(Command::Device { command }) => handle_device(&ctx, command)?,
        Some(Command::Sync { command }) => handle_sync(&ctx, command)?,
        Some(Command::Settings { command }) => handle_settings(&ctx, command)?,
        Some(Command::Mcp) => unreachable!(),
    };

    print_output(&value, cli.json).map_err(CliError::runtime)?;
    Ok(EXIT_SUCCESS)
}

fn handle_focus(ctx: &CliContext, command: FocusCommand) -> CliResult<Value> {
    let value = match command {
        FocusCommand::Get => ctx
            .runtime
            .block_on(ctx.server.cli_get_active_focus())
            .map_err(CliError::from_string)?,
        FocusCommand::Set(args) => ctx
            .runtime
            .block_on(ctx.server.cli_set_focus(args.quest_id, args.trigger))
            .map_err(CliError::from_string)?,
    };
    Ok(value)
}

fn handle_quest(ctx: &CliContext, command: QuestCommand) -> CliResult<Value> {
    let value = match command {
        QuestCommand::List(args) => ctx
            .runtime
            .block_on(ctx.server.cli_list_quests(args.space_id, args.status))
            .map_err(CliError::from_string)?,
        QuestCommand::Get(args) => ctx
            .runtime
            .block_on(ctx.server.cli_get_quest(args.id))
            .map_err(CliError::from_string)?,
        QuestCommand::Create(args) => ctx
            .runtime
            .block_on(ctx.server.cli_create_quest(
                args.title,
                args.space_id,
                args.description,
                args.due,
                args.due_time,
                args.repeat_rule,
            ))
            .map_err(CliError::from_string)?,
        QuestCommand::Update(args) => {
            let params = UpdateQuestParams {
                id: args.id,
                title: args.title,
                description: args.description,
                status: args.status,
                space_id: args.space_id,
                pinned: args.pinned,
                due: args.due,
                due_time: args.due_time,
                repeat_rule: args.repeat_rule,
                order_rank: args.order_rank,
                set_focus: args.set_focus.then_some(true),
                trigger_reminder_focus: args.trigger_reminder_focus.then_some(true),
            };
            ctx.runtime
                .block_on(ctx.server.cli_update_quest(params))
                .map_err(CliError::from_string)?
        }
        QuestCommand::Complete(args) => ctx
            .runtime
            .block_on(ctx.server.cli_complete_quest(args.id))
            .map_err(CliError::from_string)?,
        QuestCommand::Abandon(args) => ctx
            .runtime
            .block_on(ctx.server.cli_abandon_quest(args.id))
            .map_err(CliError::from_string)?,
        QuestCommand::Delete(args) => ctx
            .runtime
            .block_on(ctx.server.cli_delete_quest(args.id))
            .map_err(CliError::from_string)?,
        QuestCommand::History => ctx
            .runtime
            .block_on(ctx.server.cli_list_history())
            .map_err(CliError::from_string)?,
    };
    Ok(value)
}

fn handle_space(ctx: &CliContext, command: SpaceCommand) -> CliResult<Value> {
    let value = match command {
        SpaceCommand::List => ctx
            .runtime
            .block_on(ctx.server.cli_list_spaces())
            .map_err(CliError::from_string)?,
        SpaceCommand::Create(args) => ctx
            .runtime
            .block_on(ctx.server.cli_create_space(args.name))
            .map_err(CliError::from_string)?,
        SpaceCommand::Update(args) => ctx
            .runtime
            .block_on(ctx.server.cli_update_space(args.id, args.name))
            .map_err(CliError::from_string)?,
        SpaceCommand::Delete(args) => ctx
            .runtime
            .block_on(ctx.server.cli_delete_space(args.id))
            .map_err(CliError::from_string)?,
    };
    Ok(value)
}

fn handle_backup(ctx: &CliContext, command: BackupCommand) -> CliResult<Value> {
    let mut conn = open_db_at_path(&ctx.db_path);
    let value = match command {
        BackupCommand::Export(args) => {
            if args.all_spaces && !args.space_id.is_empty() {
                return Err(CliError::from_string(
                    "cannot combine --all-spaces with --space-id".to_string(),
                ));
            }
            let space_ids = if args.all_spaces {
                use crate::schema::spaces;
                use diesel::prelude::*;
                spaces::table
                    .select(spaces::id)
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
    let value = match command {
        ReminderCommand::List(args) => ctx
            .runtime
            .block_on(ctx.server.cli_list_reminders(args.quest_id))
            .map_err(CliError::from_string)?,
        ReminderCommand::Create(args) => ctx
            .runtime
            .block_on(ctx.server.cli_create_reminder(
                args.quest_id,
                args.kind,
                args.mm_offset,
                args.due_at_utc,
            ))
            .map_err(CliError::from_string)?,
        ReminderCommand::Delete(args) => ctx
            .runtime
            .block_on(ctx.server.cli_delete_reminder(args.id))
            .map_err(CliError::from_string)?,
    };
    Ok(value)
}

fn handle_device(ctx: &CliContext, command: DeviceCommand) -> CliResult<Value> {
    let mut conn = open_db_at_path(&ctx.db_path);
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
    let mut conn = open_db_at_path(&ctx.db_path);
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
    let mut conn = open_db_at_path(&ctx.db_path);
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
        Value::Null => {
            println!("No active Focus quest.");
        }
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
