use clap::{ArgAction, ArgGroup, Args, Parser, Subcommand};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde_json::{json, Value};

use crate::models::{
    CreateQuestInput, CreateReminderInput, CreateSpaceInput, QuestPlacement, QuestUpdatePatch,
    UpdateQuestInput, UpdateSpaceInput,
};

use crate::models::{parse_utc_timestamp, EditScope, Quest};
use crate::repositories::quest_repository::append_focus_history;
use crate::repositories::quest_repository::QuestChange;
use crate::repositories::OutboxRepository;
use crate::repositories::QuestRepository;
use crate::repositories::SpaceRepository;
use crate::services::backup;
use crate::services::bluetooth_channel_service::BluetoothChannelService;
use crate::services::channel_service::{ChannelService, LinkHandlers};
use crate::services::cli_update::{maybe_auto_update, run_update, UpdateOptions};
use crate::services::db::{db_default_path, DbConnection, DbPool, WriteTransaction};
use crate::services::device_service::DeviceService;
use crate::services::device_service::{DevicePairRequestAckInput, DevicePairRequestInput};
use crate::services::event_bus_service::EventBus;
use crate::services::network_channel_service::NetworkChannelService;
use crate::services::reminder::ReminderService;
use crate::services::settings::{self, ThemeMode};
use crate::services::space_sync_service::{SpaceResolutionMode, SpaceSyncService};
#[cfg(any(feature = "cli-plane", test))]
use chrono::Utc;

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
    #[command(about = "List a quest's checklist items (issue #128)")]
    ChecklistList(QuestIdArg),
    #[command(about = "Add an unchecked checklist item to this occurrence")]
    ChecklistAdd(ChecklistAddArgs),
    #[command(about = "Check a checklist item")]
    ChecklistCheck(ChecklistItemArgs),
    #[command(about = "Uncheck a checklist item")]
    ChecklistUncheck(ChecklistItemArgs),
    #[command(about = "Rename a checklist item")]
    ChecklistEdit(ChecklistEditArgs),
    #[command(about = "Remove a checklist item from this occurrence")]
    ChecklistRemove(ChecklistItemArgs),
    #[command(about = "List a quest's checklist audit activity")]
    ChecklistActivity(QuestIdArg),
    #[command(
        about = "Replace a quest's checklist. For a repeating quest --scope this edits only its oldest open day; --scope future edits the quest's template, which reaches every open day (ADR-0012)"
    )]
    ChecklistSetTemplate(ChecklistSetTemplateArgs),
    #[command(
        about = "Complete every open occurrence of a repeating quest dated today or before (ADR-0012)"
    )]
    CompleteThroughToday(IdArg),
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
    #[arg(long, value_parser = ["small", "medium", "large"], default_value = "medium", help = "Quest effort estimate")]
    energy: String,
    #[arg(long, value_parser = ["low", "medium", "high"], default_value = "medium", help = "Quest urgency/importance")]
    priority: String,
    #[arg(long)]
    repeat_rule: Option<String>,
    #[arg(
        long,
        value_parser = ["daily", "weekdays", "weekly", "monthly", "yearly", "null"],
        conflicts_with = "repeat_rule",
        help = "Recurring preset, or literal null"
    )]
    repeat: Option<String>,
    #[arg(
        long,
        conflicts_with = "description",
        help = "Raw task-list text for the initial checklist (e.g. '- [ ] headphones'); stored in the same field as --description, so the two are exclusive. Conflicts with --item"
    )]
    checklist: Option<String>,
    #[arg(
        long = "item",
        action = ArgAction::Append,
        conflicts_with_all = ["checklist", "description"],
        help = "Add one unchecked checklist item; repeat --item for each item. Conflicts with --description"
    )]
    items: Vec<String>,
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
    #[arg(long, help = "Text, or literal null to clear; omit to preserve")]
    due: Option<String>,
    #[arg(long, help = "Text, or literal null to clear; omit to preserve")]
    due_time: Option<String>,
    #[arg(long, value_parser = ["small", "medium", "large"], help = "Quest effort estimate; omit to preserve")]
    energy: Option<String>,
    #[arg(long, value_parser = ["low", "medium", "high"], help = "Quest urgency/importance; omit to preserve")]
    priority: Option<String>,
    #[arg(long, help = "Text, or literal null to clear; omit to preserve")]
    repeat_rule: Option<String>,
    #[arg(
        long,
        value_parser = ["daily", "weekdays", "weekly", "monthly", "yearly", "null"],
        conflicts_with = "repeat_rule",
        help = "Recurring preset, or literal null to clear; omit to preserve"
    )]
    repeat: Option<String>,
    #[arg(
        long,
        value_name = "ID",
        group = "placement",
        help = "Move the quest directly before the quest with this id, in the order of `fini quest list`"
    )]
    before: Option<String>,
    #[arg(
        long,
        value_name = "ID",
        group = "placement",
        help = "Move the quest directly after the quest with this id, in the order of `fini quest list`"
    )]
    after: Option<String>,
    #[arg(long, group = "placement", action = ArgAction::SetTrue, help = "Move the quest to the top of the list")]
    top: bool,
    #[arg(long, group = "placement", action = ArgAction::SetTrue, help = "Move the quest to the bottom of the list")]
    bottom: bool,
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
    /// The peer's iroh key, when its presence beacon has not been heard yet
    /// (a fresh CLI process has not run discovery long enough).
    #[arg(long)]
    to_endpoint_id: Option<String>,
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
    /// The peer's iroh key (hex) to pin for the pair (ADR-0009 D8). Without
    /// it the pair cannot connect until it is paired through the app.
    #[arg(long)]
    endpoint_id: Option<String>,
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
#[allow(clippy::enum_variant_names)] // The variant names are the CLI command names.
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
struct ChecklistAddArgs {
    #[arg(long)]
    quest_id: String,
    #[arg(long)]
    text: String,
}

#[derive(Args)]
struct ChecklistItemArgs {
    #[arg(long)]
    quest_id: String,
    #[arg(long)]
    item_id: String,
}

#[derive(Args)]
struct ChecklistEditArgs {
    #[arg(long)]
    quest_id: String,
    #[arg(long)]
    item_id: String,
    #[arg(long)]
    text: String,
}

#[derive(Args)]
struct ChecklistSetTemplateArgs {
    #[arg(long)]
    quest_id: String,
    #[arg(
        long,
        help = "Full replacement task-list text, e.g. '- [ ] headphones\\n- [ ] key fob'"
    )]
    checklist: String,
    #[arg(
        long,
        value_parser = ["this", "future"],
        default_value = "this",
        help = "'this' edits only the oldest open day; 'future' edits the quest's template (ADR-0012)"
    )]
    scope: String,
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
    pool: DbPool,
    devices: std::sync::Arc<DeviceService>,
    space_sync: std::sync::Arc<SpaceSyncService>,
}

impl CliContext {
    /// The CLI's composition root (ADR-0010 D15), as the app's in `lib.rs`.
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
        let pool = DbPool::open(&db_path).map_err(CliError::runtime)?;
        let identity = crate::repositories::DeviceRepository::new(
            &mut *pool.get().map_err(CliError::runtime)?,
        )
        .load_or_create_identity(&app_data_dir)
        .map_err(CliError::runtime)?;
        let bus = std::sync::Arc::new(EventBus::default());
        let network = NetworkChannelService::new(identity.clone(), pool.clone(), bus.clone());
        let bluetooth = BluetoothChannelService::new(identity.clone(), pool.clone(), bus.clone());
        let space_sync = SpaceSyncService::new(
            identity.clone(),
            pool.clone(),
            bus.clone(),
            network.clone(),
            bluetooth.clone(),
        );
        let devices = DeviceService::new(identity, pool.clone(), bus, network.clone(), bluetooth);
        network.set_link_handlers(LinkHandlers {
            peer: devices.clone(),
            session: space_sync.clone(),
        });
        // The CLI connects out for sync and cannot connect to what it has not
        // found, so it starts discovery, though it serves nobody.
        network.start_discovery();
        Ok(Self {
            pool,
            devices,
            space_sync,
        })
    }

    fn device_id(&self) -> &str {
        &self.devices.identity().device_id
    }

    fn open_db(&self) -> CliResult<DbConnection> {
        self.pool.get().map_err(CliError::runtime)
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

/// Writes one change of quest `id` and its outbox rows in one transaction
/// (docs/db.md, rules 4-6), as the app's `write_change` does, then keeps the
/// reminder in step. Returns the quest as the command shows it.
fn write_change(
    ctx: &CliContext,
    conn: &mut SqliteConnection,
    id: &str,
    write: impl FnOnce(&mut SqliteConnection) -> Result<QuestChange, String>,
) -> CliResult<Value> {
    let quest = conn
        .write_transaction(|conn| {
            let previous_space_id = QuestRepository::new(conn).get(id)?.space_id;
            let change = write(conn)?;
            OutboxRepository::new(conn).record_change(
                ctx.device_id(),
                &previous_space_id,
                &change,
            )?;
            Ok(change.quest)
        })
        .map_err(CliError::from_string)?;
    refresh_cli_reminder(conn, &quest.id);
    serde_json::to_value(quest).map_err(|e| CliError::runtime(e.to_string()))
}

/// The reminder rows follow the quest: a one-time quest as it is; a
/// repeating quest's next open occurrence on which it is still to come
/// (ADR-0012 D9).
fn refresh_cli_reminder(conn: &mut diesel::sqlite::SqliteConnection, quest_id: &str) {
    let target = (|| -> Result<Option<crate::models::Quest>, String> {
        if QuestRepository::new(conn).is_repeating(quest_id)? {
            return ReminderService::new(conn).armed_day(quest_id);
        }
        QuestRepository::new(conn).get(quest_id).map(Some)
    })();
    match target {
        Ok(Some(quest)) => sync_reminder_rows(conn, &quest),
        Ok(None) => {
            let _ = ReminderService::new(conn).delete_for_quest(quest_id);
        }
        Err(_) => {}
    }
}

fn handle_focus(ctx: &CliContext, command: FocusCommand) -> CliResult<Value> {
    let mut conn = ctx.open_db()?;
    match command {
        FocusCommand::Get => {
            let quest = QuestRepository::new(&mut conn)
                .focus(chrono::Utc::now())
                .map_err(CliError::runtime)?;
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
            let quest = QuestRepository::new(&mut conn)
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
            let quests: Vec<_> = sort_quests_for_list(
                QuestRepository::new(&mut conn)
                    .all()
                    .map_err(CliError::from_string)?,
            )
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
                .resolved(&args.id)
                .map_err(CliError::from_string)?,
        )
        .map_err(|e| CliError::runtime(e.to_string())),
        QuestCommand::Create(args) => {
            // A checklist quest stores its checklist in the same `description` field as prose
            // quests — `is_checklist` just tells the app to parse/render it as a task list
            // instead of free text (issue #128; --checklist and --item are exclusive with
            // --description at the arg level, see QuestCreateArgs).
            let (description, is_checklist) = if !args.items.is_empty() {
                let mut md: Option<String> = None;
                for item in &args.items {
                    md = Some(crate::services::checklist::add_item(md.as_deref(), item));
                }
                (md, true)
            } else if let Some(checklist) = args.checklist {
                (Some(normalize_cli_checklist(&checklist)), true)
            } else {
                (args.description, false)
            };
            let input = CreateQuestInput {
                space_id: args.space_id.unwrap_or_else(|| "1".to_string()),
                title: args.title,
                description,
                energy: crate::models::parse_energy_name(&args.energy)
                    .expect("clap restricts --energy values"),
                priority: crate::models::parse_priority_name(&args.priority)
                    .expect("clap restricts --priority values"),
                due: args.due,
                due_time: args.due_time,
                repeat_rule: args
                    .repeat
                    .as_deref()
                    .filter(|repeat| *repeat != "null")
                    .map(normalize_repeat_alias)
                    .or(args.repeat_rule),
                order_rank: None,
                is_checklist,
            };
            let quest = conn
                .write_transaction(|conn| {
                    let quest = QuestRepository::new(conn).create(input)?;
                    OutboxRepository::new(conn).record_quest(
                        ctx.device_id(),
                        &quest.space_id,
                        &quest.id,
                    )?;
                    Ok(quest)
                })
                .map_err(CliError::from_string)?;
            refresh_cli_reminder(&mut conn, &quest.id);
            serde_json::to_value(quest).map_err(|e| CliError::runtime(e.to_string()))
        }
        QuestCommand::Update(args) => update_quest_from_cli(ctx, &mut conn, args),
        QuestCommand::Complete(args) => update_quest_status(ctx, &mut conn, args.id, "completed"),
        QuestCommand::Abandon(args) => update_quest_status(ctx, &mut conn, args.id, "abandoned"),
        QuestCommand::Delete(args) => {
            conn.write_transaction(|conn| {
                let quest = QuestRepository::new(conn).get(&args.id)?;
                OutboxRepository::new(conn).emit_sync_event(
                    ctx.device_id(),
                    "quest",
                    &args.id,
                    &quest.space_id,
                    "delete",
                    None,
                )?;
                QuestRepository::new(conn).delete(&args.id).map(|_| ())
            })
            .map_err(CliError::from_string)?;
            let _ = ReminderService::new(&mut conn).delete_for_quest(&args.id);
            Ok(json!({ "deleted": true, "id": args.id }))
        }
        QuestCommand::History => {
            let mut quests: Vec<_> = QuestRepository::new(&mut conn)
                .all()
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
        QuestCommand::ChecklistList(args) => {
            let quest = QuestRepository::new(&mut conn)
                .resolved(&args.quest_id)
                .map_err(CliError::from_string)?;
            let items = crate::services::checklist::parse_opt(quest.description.as_deref());
            Ok(
                json!({ "quest_id": args.quest_id, "is_checklist": quest.is_checklist, "items": items }),
            )
        }
        QuestCommand::ChecklistAdd(args) => write_change(ctx, &mut conn, &args.quest_id, |conn| {
            QuestRepository::new(conn).add_checklist_item(
                &args.quest_id,
                &args.text,
                EditScope::All,
                Some(ctx.device_id()),
            )
        }),
        QuestCommand::ChecklistCheck(args) => {
            write_change(ctx, &mut conn, &args.quest_id, |conn| {
                QuestRepository::new(conn).toggle_checklist_item(
                    &args.quest_id,
                    &args.item_id,
                    true,
                    None,
                )
            })
        }
        QuestCommand::ChecklistUncheck(args) => {
            write_change(ctx, &mut conn, &args.quest_id, |conn| {
                QuestRepository::new(conn).toggle_checklist_item(
                    &args.quest_id,
                    &args.item_id,
                    false,
                    None,
                )
            })
        }
        QuestCommand::ChecklistEdit(args) => write_change(ctx, &mut conn, &args.quest_id, |conn| {
            QuestRepository::new(conn).edit_checklist_item_text(
                &args.quest_id,
                &args.item_id,
                &args.text,
                EditScope::All,
                Some(ctx.device_id()),
            )
        }),
        QuestCommand::ChecklistRemove(args) => {
            write_change(ctx, &mut conn, &args.quest_id, |conn| {
                QuestRepository::new(conn).remove_checklist_item(
                    &args.quest_id,
                    &args.item_id,
                    EditScope::All,
                    Some(ctx.device_id()),
                )
            })
        }
        QuestCommand::CompleteThroughToday(args) => {
            write_change(ctx, &mut conn, &args.id, |conn| {
                QuestRepository::new(conn).complete_through_today(&args.id, Some(ctx.device_id()))
            })
        }
        QuestCommand::ChecklistActivity(args) => {
            let activity = QuestRepository::new(&mut conn)
                .get_checklist_activity(&args.quest_id)
                .map_err(CliError::from_string)?;
            Ok(json!({ "quest_id": args.quest_id, "activity": activity }))
        }
        QuestCommand::ChecklistSetTemplate(args) => {
            let scope = if args.scope == "future" {
                EditScope::All
            } else {
                EditScope::Day
            };
            // Text-matching lines keep the ids of the quest's template, so an
            // unchanged item keeps its identity (and its ticks on open days).
            let template = QuestRepository::new(&mut conn)
                .get(&args.quest_id)
                .map_err(CliError::from_string)?
                .description;
            let normalized_checklist =
                normalize_future_template(&args.checklist, template.as_deref());
            write_change(ctx, &mut conn, &args.quest_id, |conn| {
                QuestRepository::new(conn).set_checklist(
                    &args.quest_id,
                    &normalized_checklist,
                    scope,
                    Some(ctx.device_id()),
                )
            })
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

fn normalize_repeat_alias(value: &str) -> String {
    match value {
        preset @ ("daily" | "weekdays" | "weekly" | "monthly" | "yearly") => {
            json!({ "preset": preset }).to_string()
        }
        "null" => "null".to_string(),
        _ => unreachable!("clap restricts --repeat to supported aliases"),
    }
}

fn normalize_cli_checklist(value: &str) -> String {
    crate::services::checklist::serialize(&crate::services::checklist::parse(value))
}

/// Like `normalize_cli_checklist`, but for `checklist-set-template --scope future`:
/// the documented raw-text input (`'- [ ] headphones'`, no `<!--k=...-->` tokens) would otherwise
/// get a fresh random id per line, which `reconcile_future_scope` then can't match against the
/// series template's existing item ids — silently resetting an already-checked item back to
/// unchecked even though the user only meant to keep it. Reuse the series template's id for
/// any line whose text exactly matches an existing item, so unchanged lines keep their identity.
fn normalize_future_template(raw: &str, series_template_checklist: Option<&str>) -> String {
    use std::collections::{HashMap, VecDeque};

    let existing = crate::services::checklist::parse_opt(series_template_checklist);
    // A queue per text, not a single id: two existing items can share the same text, and two raw
    // lines can too. Popping one id per match pairs them up one-to-one in original order instead
    // of handing every matching line the same id (which would give duplicate items the same
    // <!--k=id--> token — breaking per-item identity for both removal-by-id and sync merge).
    let mut existing_by_text: HashMap<&str, VecDeque<&str>> = HashMap::new();
    for item in &existing {
        existing_by_text
            .entry(item.text.as_str())
            .or_default()
            .push_back(item.id.as_str());
    }

    let items: Vec<crate::services::checklist::ChecklistItem> =
        crate::services::checklist::parse(raw)
            .into_iter()
            .map(|mut item| {
                if let Some(existing_id) = existing_by_text
                    .get_mut(item.text.as_str())
                    .and_then(VecDeque::pop_front)
                {
                    item.id = existing_id.to_string();
                }
                item
            })
            .collect();
    crate::services::checklist::serialize(&items)
}

fn update_quest_from_cli(
    ctx: &CliContext,
    conn: &mut diesel::sqlite::SqliteConnection,
    args: QuestUpdateArgs,
) -> CliResult<Value> {
    let placement = args.placement();
    let description = parse_nullable_quest_patch(args.description);
    let due = parse_nullable_quest_patch(args.due);
    let due_time = parse_nullable_quest_patch(args.due_time);
    let repeat_rule = parse_nullable_quest_patch(
        args.repeat
            .as_deref()
            .map(normalize_repeat_alias)
            .or(args.repeat_rule),
    );
    let input = UpdateQuestInput {
        space_id: args.space_id,
        title: args.title,
        description: None,
        status: args.status.clone(),
        energy: args.energy.as_deref().map(|value| {
            crate::models::parse_energy_name(value).expect("clap restricts --energy values")
        }),
        priority: args.priority.as_deref().map(|value| {
            crate::models::parse_priority_name(value).expect("clap restricts --priority values")
        }),
        due: None,
        due_time: None,
        repeat_rule: None,
        is_checklist: None,
    };
    let patch = QuestUpdatePatch {
        input,
        description,
        due,
        due_time,
        repeat_rule,
    };
    let quest = write_change(ctx, conn, &args.id, |conn| {
        let change = QuestRepository::new(conn).update(
            &args.id,
            patch,
            EditScope::All,
            None,
            Some(ctx.device_id()),
        )?;
        if args.trigger_reminder_focus {
            append_focus_history(conn, &change.quest.id, &change.quest.space_id, "reminder")?;
        } else if args.set_focus
            || args.status.as_deref() == Some("active")
            || change.restore_should_focus
        {
            append_focus_history(conn, &change.quest.id, &change.quest.space_id, "manual")?;
        }
        Ok(change)
    })?;
    let Some(placement) = placement else {
        return Ok(quest);
    };
    let moved = conn
        .write_transaction(|conn| {
            let written = QuestRepository::new(conn).move_quest(&args.id, &placement)?;
            OutboxRepository::new(conn).record_quests(ctx.device_id(), &written)?;
            QuestRepository::new(conn).resolved(&args.id)
        })
        .map_err(CliError::from_string)?;
    serde_json::to_value(moved).map_err(|e| CliError::runtime(e.to_string()))
}

impl QuestUpdateArgs {
    /// The move that `--before`, `--after`, `--top` or `--bottom` gives
    /// (clap lets only one of them through).
    fn placement(&self) -> Option<QuestPlacement> {
        if let Some(id) = &self.before {
            return Some(QuestPlacement::Before(id.clone()));
        }
        if let Some(id) = &self.after {
            return Some(QuestPlacement::After(id.clone()));
        }
        if self.top {
            return Some(QuestPlacement::Top);
        }
        self.bottom.then_some(QuestPlacement::Bottom)
    }
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
            due: None,
            due_time: None,
            energy: None,
            priority: None,
            repeat_rule: None,
            repeat: None,
            before: None,
            after: None,
            top: false,
            bottom: false,
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
                &mut *ctx.open_db()?,
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

fn to_json<T: serde::Serialize>(value: T) -> CliResult<Value> {
    serde_json::to_value(value).map_err(|e| CliError::runtime(e.to_string()))
}

/// An arm that does SQL takes its own connection; an arm that calls a
/// service lets the service take one (docs/db.md, rule 2).
fn handle_device(ctx: &CliContext, command: DeviceCommand) -> CliResult<Value> {
    let devices = &ctx.devices;
    let value = match command {
        DeviceCommand::Identity => to_json(devices.identity())?,
        DeviceCommand::AddMode { command } => {
            let on = matches!(command, DeviceAddModeCommand::Enter);
            devices
                .set_add_device_mode(on, true)
                .map_err(CliError::from_string)?;
            json!({ "ok": true })
        }
        DeviceCommand::Discovery | DeviceCommand::Presence => to_json(devices.heard_devices())?,
        DeviceCommand::Pair { command } => handle_device_pair(ctx, command)?,
        DeviceCommand::Paired { command } => match command {
            DevicePairedCommand::List => {
                to_json(devices.paired_devices().map_err(CliError::from_string)?)?
            }
            DevicePairedCommand::Save(args) => to_json(
                devices
                    .save_paired_device(
                        &mut *ctx.open_db()?,
                        &args.peer_device_id,
                        &args.display_name,
                        None,
                        false,
                        None,
                        args.endpoint_id,
                    )
                    .map_err(CliError::from_string)?,
            )?,
            DevicePairedCommand::UpdateLastSeen(args) => {
                devices
                    .update_last_seen(
                        &mut *ctx.open_db()?,
                        &args.peer_device_id,
                        &args.last_seen_at,
                    )
                    .map_err(CliError::from_string)?;
                json!({ "ok": true })
            }
            DevicePairedCommand::Unpair(args) => {
                devices
                    .unpair(&mut *ctx.open_db()?, &args.peer_device_id)
                    .map_err(CliError::from_string)?;
                json!({ "ok": true })
            }
        },
        DeviceCommand::Updates { command } => match command {
            DeviceUpdatesCommand::ConsumeSpaceMapping => {
                to_json(ctx.space_sync.take_space_mapping_updates())?
            }
        },
        DeviceCommand::Debug { command } => match command {
            DeviceDebugCommand::Status => {
                to_json(devices.debug_status().map_err(CliError::from_string)?)?
            }
        },
    };
    Ok(value)
}

fn handle_device_pair(ctx: &CliContext, command: DevicePairCommand) -> CliResult<Value> {
    let devices = &ctx.devices;
    let value = match command {
        DevicePairCommand::Send(args) => {
            devices
                .send_pair_request(DevicePairRequestInput {
                    request_id: args.request_id,
                    to_device_id: args.to_device_id,
                    to_addr: args.to_addr,
                    to_ws_port: args.to_ws_port,
                    to_endpoint_id: args.to_endpoint_id,
                })
                .map_err(CliError::from_string)?;
            json!({ "ok": true })
        }
        DevicePairCommand::Incoming => {
            to_json(devices.incoming_requests().map_err(CliError::from_string)?)?
        }
        DevicePairCommand::OutgoingUpdates => {
            to_json(devices.outgoing_updates().map_err(CliError::from_string)?)?
        }
        DevicePairCommand::OutgoingCompletions => to_json(
            devices
                .outgoing_completions()
                .map_err(CliError::from_string)?,
        )?,
        DevicePairCommand::Accept(args) => to_json(
            devices
                .accept_pair_request(DevicePairRequestAckInput {
                    request_id: args.request_id,
                })
                .map_err(CliError::from_string)?,
        )?,
        DevicePairCommand::Complete(args) => {
            devices
                .complete_pair_request(DevicePairRequestAckInput {
                    request_id: args.request_id,
                })
                .map_err(CliError::from_string)?;
            json!({ "ok": true })
        }
        DevicePairCommand::Acknowledge(args) => {
            devices
                .acknowledge_pair_request(DevicePairRequestAckInput {
                    request_id: args.request_id,
                })
                .map_err(CliError::from_string)?;
            json!({ "ok": true })
        }
    };
    Ok(value)
}

/// As `handle_device`: `tick` takes its own connections.
fn handle_sync(ctx: &CliContext, command: SyncCommand) -> CliResult<Value> {
    let space_sync = &ctx.space_sync;
    let value = match command {
        SyncCommand::Mappings { command } => match command {
            SyncMappingsCommand::List(args) => to_json(
                space_sync
                    .list_mappings(&mut *ctx.open_db()?, &args.peer_device_id)
                    .map_err(CliError::from_string)?,
            )?,
            SyncMappingsCommand::Update(args) => to_json(
                space_sync
                    .update_mappings(
                        &mut *ctx.open_db()?,
                        &args.peer_device_id,
                        args.mapped_space_id,
                    )
                    .map_err(CliError::from_string)?,
            )?,
            SyncMappingsCommand::ApplyRemote(args) => to_json(
                space_sync
                    .apply_remote_mappings(
                        &mut *ctx.open_db()?,
                        &args.peer_device_id,
                        args.mapped_space_id,
                        Vec::new(),
                    )
                    .map_err(CliError::from_string)?,
            )?,
            SyncMappingsCommand::ResolveCustom(args) => to_json(
                space_sync
                    .resolve_custom_space_mapping(
                        &mut *ctx.open_db()?,
                        &args.peer_device_id,
                        args.space_id,
                        args.space_name,
                        parse_space_resolution_mode(&args.mode)?,
                        args.existing_local_space_id,
                    )
                    .map_err(CliError::from_string)?,
            )?,
        },
        SyncCommand::Tick { peer_device_id } => {
            let result = space_sync.tick().map_err(CliError::from_string)?;
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
                to_json(result)?
            }
        }
        SyncCommand::Status { peer_device_id } => to_json(
            space_sync
                .status(&mut *ctx.open_db()?, peer_device_id)
                .map_err(CliError::from_string)?,
        )?,
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

#[allow(clippy::print_stdout, reason = "the CLI writes its result to stdout")]
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
                .map(|(key, value)| match value {
                    Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                        format!("{key}={}", format_scalar(value))
                    }
                    Value::Array(items) => format!("{key}={}", format_count("item", items.len())),
                    Value::Object(nested) => format!("{key}={} fields", nested.len()),
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

/// The order of `fini quest list`: the active quests in the order of the
/// UI lists (`Quest::list_order`, the same as `activeOrder` in
/// src/stores/quest.ts), then the other quests, the newest change first.
#[cfg(any(feature = "cli-plane", test))]
pub(crate) fn sort_quests_for_list(mut loaded: Vec<Quest>) -> Vec<Quest> {
    let now = Utc::now();
    loaded.sort_by(|a, b| {
        let a_active = a.status == "active";
        let b_active = b.status == "active";
        if a_active != b_active {
            return b_active.cmp(&a_active);
        }

        if a_active {
            return a.list_order(b, &now);
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

#[cfg(test)]
#[path = "cli/tests.rs"]
mod tests;
