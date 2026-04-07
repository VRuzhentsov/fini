use clap::{ArgAction, Args, Parser, Subcommand};
use serde_json::Value;

use crate::services::db::db_default_path;
use crate::services::mcp::{FiniServer, UpdateQuestParams};

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
    App,
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
}

#[derive(Subcommand)]
enum DevicePairedCommand {
    List,
    Save(DevicePairedSaveArgs),
    Unpair(PeerDeviceIdArg),
}

#[derive(Args)]
struct DevicePairedSaveArgs {
    #[arg(long)]
    peer_device_id: String,
    #[arg(long)]
    display_name: String,
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

        Ok(Self {
            server: FiniServer::new(&db_path),
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

    if let Some(Command::App) = cli.command {
        crate::run();
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
        Some(Command::Reminder { command }) => handle_reminder(&ctx, command)?,
        Some(Command::Device { command: _ }) => {
            return Err(CliError::runtime(
                "device command group is not implemented yet in CLI runtime",
            ));
        }
        Some(Command::Sync { command: _ }) => {
            return Err(CliError::runtime(
                "sync command group is not implemented yet in CLI runtime",
            ));
        }
        Some(Command::App) | Some(Command::Mcp) => unreachable!(),
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
