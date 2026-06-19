use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::config_support::default_config_path_string;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "garyx", version = VERSION, about = "Garyx – AI chat gateway")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,

    // Shared gateway flags used by `gateway run` and `onboard --run-gateway`.
    /// Config file path
    #[arg(short, long, default_value_t = default_config_path_string(), global = true)]
    pub(crate) config: String,

    /// Override gateway port
    #[arg(short, long, hide = true)]
    pub(crate) port: Option<u16>,

    /// Override gateway host
    #[arg(long, hide = true)]
    pub(crate) host: Option<String>,

    /// Start without channel polling
    #[arg(long, hide = true)]
    pub(crate) no_channels: bool,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Gateway management
    #[command(alias = "gw")]
    Gateway {
        #[command(subcommand)]
        action: GatewayAction,
    },
    /// Configuration utilities
    #[command(alias = "cfg")]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Command list and prompt shortcut management
    #[command(
        name = "commands",
        visible_alias = "shortcuts",
        visible_alias = "shortcut"
    )]
    CommandList {
        #[command(subcommand)]
        action: CommandAction,
    },
    /// Show running status
    #[command(alias = "ps")]
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Run health checks
    #[command(alias = "check")]
    Doctor {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Guided setup for a new config
    #[command(alias = "setup")]
    Onboard {
        /// Overwrite existing config file
        #[arg(long)]
        force: bool,
        /// API account id to create or enable for gateway/API usage
        #[arg(long, default_value = "main")]
        api_account: String,
        /// Search API key to persist in gateway.search.api_key
        #[arg(long)]
        search_api_key: Option<String>,
        /// OpenAI API key to persist in gateway.conversation_index.api_key
        #[arg(long)]
        conversation_index_api_key: Option<String>,
        /// Enable conversation vector indexing
        #[arg(long, conflicts_with = "disable_conversation_index")]
        enable_conversation_index: bool,
        /// Disable conversation vector indexing
        #[arg(long, conflicts_with = "enable_conversation_index")]
        disable_conversation_index: bool,
        /// Override conversation index model
        #[arg(long)]
        conversation_index_model: Option<String>,
        /// Override conversation index base URL
        #[arg(long)]
        conversation_index_base_url: Option<String>,
        /// Start the gateway after onboarding completes
        #[arg(long)]
        run_gateway: bool,
        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },
    /// Download and replace the current garyx binary from GitHub Releases
    #[command(visible_alias = "upgrade")]
    Update {
        /// Specific version to install (defaults to latest release)
        #[arg(long)]
        version: Option<String>,
        /// Override the target binary path (defaults to the current executable)
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Manage the gateway + plugin auto-update kill switches
    #[command(name = "auto-update")]
    AutoUpdate {
        #[command(subcommand)]
        action: AutoUpdateAction,
    },
    /// Channel account management
    #[command(alias = "channel")]
    Channels {
        #[command(subcommand)]
        action: ChannelsAction,
    },
    /// Subprocess channel plugin management
    #[command(alias = "plugin")]
    Plugins {
        #[command(subcommand)]
        action: PluginsAction,
    },
    /// Local log file utilities
    #[command(alias = "log")]
    Logs {
        #[command(subcommand)]
        action: LogsAction,
    },
    /// Bot status and current binding utilities
    #[command(name = "bot")]
    Bot {
        #[command(subcommand)]
        action: BotAction,
    },
    /// Automation management
    #[command(name = "automation")]
    Automation {
        #[command(subcommand)]
        action: AutomationAction,
    },
    /// Dynamic workflow runs
    #[command(name = "workflow", alias = "workflows")]
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction,
    },
    /// Agent-friendly application database
    #[command(name = "db", visible_alias = "database")]
    Db {
        #[command(subcommand)]
        action: DbAction,
    },
    /// Custom agent management
    #[command(name = "agent", alias = "agents", visible_alias = "custom-agent")]
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Provider-backed utility commands
    #[command(name = "tool", visible_alias = "tools")]
    Tool {
        #[command(subcommand)]
        action: ToolAction,
    },
    /// Team asset management
    #[command(name = "team", alias = "teams")]
    Team {
        #[command(subcommand)]
        action: TeamAction,
    },
    /// Thread utilities
    #[command(alias = "threads")]
    Thread {
        #[command(subcommand)]
        action: ThreadAction,
    },
    /// Dream topic map across recent threads
    #[command(alias = "dreams")]
    Dream {
        #[command(subcommand)]
        action: DreamAction,
    },
    /// Task overlay utilities
    #[command(alias = "tasks")]
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Send an outbound channel message via a bot
    #[command(alias = "send", alias = "msg")]
    Message {
        /// Bot selector: `channel:account_id`, e.g. `telegram:main`
        #[arg(short, long)]
        bot: Option<String>,
        /// Local image path to send. Message text is used as the caption.
        #[arg(long)]
        image: Option<PathBuf>,
        /// Local file path to send. Message text is used as the caption.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Message text. Required unless --image or --file is provided.
        text: Vec<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum AutoUpdateAction {
    /// Show whether gateway + plugin auto-update are enabled and the
    /// installed / latest-known versions
    Status {
        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },
    /// Disable auto-update. By default both gateway and plugin
    /// loops are disabled; pass `--gateway` or `--plugin` to
    /// disable only one. Persists to garyx.json and triggers a
    /// gateway config reload so the loops stop on the next tick.
    Disable {
        /// Disable only the gateway auto-update loop
        #[arg(long, conflicts_with = "plugin")]
        gateway: bool,
        /// Disable only the plugin auto-update loop
        #[arg(long, conflicts_with = "gateway")]
        plugin: bool,
    },
    /// Enable auto-update. Mirror of `disable`.
    Enable {
        /// Enable only the gateway auto-update loop
        #[arg(long, conflicts_with = "plugin")]
        gateway: bool,
        /// Enable only the plugin auto-update loop
        #[arg(long, conflicts_with = "gateway")]
        plugin: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum GatewayAction {
    /// Run the gateway in the foreground (blocks until killed)
    Run {
        /// Override gateway port
        #[arg(short, long)]
        port: Option<u16>,
        /// Override gateway host
        #[arg(long)]
        host: Option<String>,
        /// Start without channel polling
        #[arg(long)]
        no_channels: bool,
    },
    /// Register the gateway with the system service manager (launchd on macOS,
    /// systemd --user on Linux) and start it. Safe to re-run to refresh config.
    Install,
    /// Stop the managed gateway and remove its unit / plist file
    Uninstall,
    /// Start the already-installed managed gateway service
    Start,
    /// Restart the managed gateway service (refreshes the unit / plist file first)
    #[command(
        long_about = "Restart the managed gateway service and refresh its unit/plist first.\n\nAgent safety: if you are running inside an agent thread, do not use a bare restart. Queue a wake so the new gateway can resume this same thread after the service comes back:\n  garyx gateway restart --wake thread <thread_id> --wake-message \"continue\"\n\nUse --no-wake only when you intentionally want the gateway to restart without resuming any agent thread."
    )]
    Restart {
        /// Wake a target after restart: `thread <thread_id>`, `task <task_id>`, or `bot <channel:account_id>`
        #[arg(
            long,
            value_names = ["KIND", "TARGET"],
            num_args = 2,
            requires = "wake_message",
            conflicts_with = "no_wake"
        )]
        wake: Vec<String>,
        /// Message to send to the wake target after the gateway is healthy
        #[arg(long, value_name = "MESSAGE")]
        wake_message: Option<String>,
        /// Intentionally restart without resuming any thread; agents should only use this when no continuation is needed
        #[arg(long = "no-wake")]
        no_wake: bool,
        /// Output raw JSON events for the wake run
        #[arg(long, requires = "wake")]
        wake_json: bool,
    },
    /// Stop the managed gateway service
    Stop,
    /// Reload the running gateway config from disk without restart
    ReloadConfig,
    /// Ensure a gateway auth token exists and print it
    Token {
        /// Generate a fresh token even if one already exists
        #[arg(long)]
        rotate: bool,
        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigAction {
    /// Print absolute config path
    Path,
    /// Get value by dotted path (e.g. gateway.port)
    Get {
        /// Dotted JSON path
        path: String,
    },
    /// Set value by dotted path
    Set {
        /// Dotted JSON path
        path: String,
        /// JSON value (falls back to string if invalid JSON)
        value: String,
    },
    /// Remove key by dotted path
    Unset {
        /// Dotted JSON path
        path: String,
    },
    /// Configure which CLI the Claude Agent SDK launches
    ClaudeCli {
        /// SDK CLI mode: cctty uses Garyx's embedded TTY wrapper; native uses Claude Code directly
        #[arg(long, value_parser = ["cctty", "native"])]
        mode: Option<String>,
        /// Explicit CLI path for the selected mode
        #[arg(long)]
        path: Option<String>,
        /// Clear the explicit CLI path
        #[arg(long, conflicts_with = "path")]
        clear_path: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Set the gateway default model for a model provider
    ProviderModel {
        /// Provider type: claude_code, codex_app_server, traex, gemini_cli, gpt, anthropic, or google
        provider: String,
        /// Default model id. Omit to leave unchanged.
        #[arg(long, conflicts_with = "clear_model")]
        model: Option<String>,
        /// Clear the configured default model.
        #[arg(long)]
        clear_model: bool,
        /// Default reasoning effort / thinking level. Omit to leave unchanged.
        #[arg(long, conflicts_with = "clear_model_reasoning_effort")]
        model_reasoning_effort: Option<String>,
        /// Clear the configured default reasoning effort.
        #[arg(long)]
        clear_model_reasoning_effort: bool,
        /// Claude Code CLI mode. Only valid when provider is claude_code.
        #[arg(
            long,
            value_parser = ["cctty", "native"],
            conflicts_with = "clear_claude_cli_mode"
        )]
        claude_cli_mode: Option<String>,
        /// Clear the configured Claude Code CLI mode.
        #[arg(long)]
        clear_claude_cli_mode: bool,
        /// Explicit Claude Code CLI path. Only valid when provider is claude_code.
        #[arg(long, conflicts_with = "clear_claude_cli_path")]
        claude_cli_path: Option<String>,
        /// Clear the configured Claude Code CLI path.
        #[arg(long)]
        clear_claude_cli_path: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Initialize config file from defaults
    Init {
        /// Overwrite existing config file
        #[arg(long)]
        force: bool,
    },
    /// Display loaded config (pretty JSON)
    Show,
    /// Validate config file
    Validate,
}

#[derive(Subcommand)]
pub(crate) enum CommandAction {
    /// List commands or shortcuts
    #[command(visible_alias = "ls")]
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Surface filter: router | gateway_api | desktop_composer | telegram | api_chat | plugin
        #[arg(long)]
        surface: Option<String>,
        /// Channel filter, e.g. telegram or feishu
        #[arg(long)]
        channel: Option<String>,
        /// Account id filter
        #[arg(long = "account-id")]
        account_id: Option<String>,
        /// Include hidden commands
        #[arg(long)]
        include_hidden: bool,
    },
    /// Show one prompt shortcut
    Get {
        /// Command name, with or without leading slash
        name: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create or update a prompt shortcut
    #[command(
        visible_alias = "add",
        visible_alias = "create",
        visible_alias = "upsert"
    )]
    Set {
        /// Command name, with or without leading slash
        name: String,
        /// Prompt text. If omitted, reads from stdin.
        #[arg(long)]
        prompt: Option<String>,
        /// Human-readable description
        #[arg(long)]
        description: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete a prompt shortcut
    #[command(visible_alias = "rm", visible_alias = "remove")]
    Delete {
        /// Command name, with or without leading slash
        name: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ToolAction {
    /// Generate exactly one image with the configured Codex provider
    Image {
        /// Image prompt
        prompt: String,
        /// Output file path
        #[arg(short, long)]
        output: PathBuf,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Wait up to this many seconds for image generation
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    /// Search the web through Gemini provider-native search
    Search {
        /// Search query
        #[arg(required = true)]
        query: Vec<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Timeout in seconds
        #[arg(long, default_value_t = 300)]
        timeout: u64,
    },
}

#[derive(Subcommand)]
pub(crate) enum ChannelsAction {
    /// List configured channel accounts
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Alias of list
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Enable an existing account
    Enable {
        /// Channel type: telegram | discord | feishu | weixin | api | <plugin_id>
        channel: String,
        /// Account id
        account: String,
    },
    /// Disable an existing account
    Disable {
        /// Channel type: telegram | discord | feishu | weixin | api | <plugin_id>
        channel: String,
        /// Account id
        account: String,
    },
    /// Remove an existing account
    Remove {
        /// Channel type: telegram | discord | feishu | weixin | api | <plugin_id>
        channel: String,
        /// Account id
        account: String,
    },
    /// Add a new account
    Add {
        /// Channel type: telegram | discord | feishu | weixin | api | <plugin_id>
        channel: Option<String>,
        /// Account id
        account: Option<String>,
        /// Friendly display name
        #[arg(long)]
        name: Option<String>,
        /// Workspace directory
        #[arg(long)]
        workspace_dir: Option<String>,
        /// Workspace mode for new bot threads: local or worktree
        #[arg(long)]
        workspace_mode: Option<String>,
        /// Agent or team id to bind this channel account to
        #[arg(long)]
        agent_id: Option<String>,
        /// Telegram or Discord bot token (for plugin-owned channels, prefer the
        /// desktop UI or `garyx plugins install` flow)
        #[arg(long)]
        token: Option<String>,
        /// Weixin UIN (optional; auto-generated when omitted)
        #[arg(long)]
        uin: Option<String>,
        /// Weixin API base URL
        #[arg(long)]
        base_url: Option<String>,
        /// Feishu app id
        #[arg(long)]
        app_id: Option<String>,
        /// Feishu app secret
        #[arg(long)]
        app_secret: Option<String>,
        /// Feishu tenant brand: feishu (国内, default) | lark (海外)
        #[arg(long)]
        domain: Option<String>,
        /// Feishu only: run the one-click device-flow registration to
        /// auto-fetch App ID / Secret. Requires a TTY for QR display.
        #[arg(long, default_value_t = false)]
        auto_register: bool,
    },
    /// Channel login helpers
    Login {
        /// Channel type: feishu | weixin | <plugin_id with auth_flows>
        channel: String,
        /// Account id to write into config (defaults to scanned bot id or app_id)
        #[arg(long)]
        account: Option<String>,
        /// Existing account id to re-authorize.
        ///
        /// Metadata such as name, workspace, agent binding, and channel
        /// specific fields are inherited unless explicitly overridden. If
        /// the provider returns a different account id, the previous id
        /// is disabled by default.
        #[arg(long)]
        reauthorize: Option<String>,
        /// Forget the previous account after the new login is saved.
        ///
        /// Without this flag, the previous account is left in config but
        /// disabled so rollback is possible.
        #[arg(long, default_value_t = false)]
        forget_previous: bool,
        /// Friendly display name
        #[arg(long)]
        name: Option<String>,
        /// Workspace directory
        #[arg(long)]
        workspace_dir: Option<String>,
        /// Workspace mode for new bot threads: local or worktree
        #[arg(long)]
        workspace_mode: Option<String>,
        /// Agent or team id to bind this channel account to
        #[arg(long)]
        agent_id: Option<String>,
        /// Weixin UIN (optional; inherited from --reauthorize when omitted)
        #[arg(long)]
        uin: Option<String>,
        /// Weixin API base URL
        #[arg(long)]
        base_url: Option<String>,
        /// Feishu tenant brand: feishu (国内, default) | lark (海外)
        #[arg(long)]
        domain: Option<String>,
        /// Login timeout in seconds
        #[arg(long, default_value_t = 480)]
        timeout_seconds: u64,
        /// Emit machine-readable JSON events and final summary.
        ///
        /// QR display payloads are printed as JSON instead of terminal block
        /// art, which lets an agent forward or render them without scraping.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum LogsAction {
    /// Print log file path
    Path {
        /// Optional override path
        #[arg(long)]
        path: Option<String>,
    },
    /// Print tail lines from log file
    Tail {
        /// Optional override path
        #[arg(long)]
        path: Option<String>,
        /// Number of lines
        #[arg(long, default_value_t = 100)]
        lines: usize,
        /// Optional substring filter
        #[arg(long)]
        pattern: Option<String>,
        /// Follow appended lines
        #[arg(long)]
        follow: bool,
    },
    /// Clear log file contents
    Clear {
        /// Optional override path
        #[arg(long)]
        path: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum BotAction {
    /// Show the bot's current main endpoint and bound thread
    Status {
        /// Bot id like telegram:main
        bot_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Exact channel endpoint binding utilities
    #[command(alias = "endpoints")]
    Endpoint {
        #[command(subcommand)]
        action: BotEndpointAction,
    },
}

#[derive(Subcommand)]
pub(crate) enum BotEndpointAction {
    /// List known channel endpoints
    List {
        /// Optional bot selector like telegram:main
        #[arg(long)]
        bot: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Bind or rebind an exact channel endpoint to an existing thread
    Bind {
        /// Endpoint key like telegram::main::123 or discord::main::456
        #[arg(long)]
        endpoint: String,
        /// Canonical thread id like thread::abc
        #[arg(long)]
        thread: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Detach an exact channel endpoint from its current thread
    #[command(alias = "unbind")]
    Detach {
        /// Endpoint key like telegram::main::123 or discord::main::456
        #[arg(long)]
        endpoint: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Default, Args)]
pub(crate) struct AutomationScheduleArgs {
    /// Run every N hours
    #[arg(long)]
    pub(crate) every_hours: Option<u64>,
    /// Run daily at HH:MM
    #[arg(long)]
    pub(crate) daily_time: Option<String>,
    /// Weekday for daily schedules: mon, tue, wed, thu, fri, sat, sun. Repeat to select multiple days. Omit for every day.
    #[arg(long = "weekday")]
    pub(crate) weekdays: Vec<String>,
    /// Timezone for daily schedules. Defaults to Asia/Shanghai when --daily-time is used.
    #[arg(long)]
    pub(crate) timezone: Option<String>,
    /// Run once at YYYY-MM-DDTHH:MM, RFC3339, or ONCE:YYYY-MM-DD HH:MM
    #[arg(long)]
    pub(crate) once_at: Option<String>,
    /// Raw AutomationScheduleView JSON, e.g. '{"kind":"interval","hours":6}'
    #[arg(long)]
    pub(crate) schedule_json: Option<String>,
}

#[derive(Subcommand)]
pub(crate) enum AutomationAction {
    /// List scheduled automations
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Get one scheduled automation
    Get {
        automation_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create a scheduled automation
    #[command(visible_alias = "add")]
    Create {
        /// Human-readable automation name
        #[arg(long, alias = "name")]
        label: String,
        /// Prompt text. If omitted, reads from stdin.
        #[arg(long)]
        prompt: Option<String>,
        /// Agent or team id to run
        #[arg(long)]
        agent_id: Option<String>,
        /// Workspace directory for the automation thread; defaults to the current directory
        #[arg(long)]
        workspace_dir: Option<String>,
        /// Existing thread to receive each scheduled prompt
        #[arg(long)]
        thread_id: Option<String>,
        #[command(flatten)]
        schedule: AutomationScheduleArgs,
        /// Create disabled, then enable later with `garyx automation resume`
        #[arg(long)]
        disabled: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Update a scheduled automation
    Update {
        automation_id: String,
        /// Human-readable automation name
        #[arg(long, alias = "name")]
        label: Option<String>,
        /// Prompt text
        #[arg(long)]
        prompt: Option<String>,
        /// Agent or team id to run
        #[arg(long)]
        agent_id: Option<String>,
        /// Workspace directory for the automation thread
        #[arg(long)]
        workspace_dir: Option<String>,
        /// Existing thread to receive each scheduled prompt
        #[arg(long)]
        thread_id: Option<String>,
        #[command(flatten)]
        schedule: AutomationScheduleArgs,
        /// Enable the automation
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        /// Disable the automation
        #[arg(long, conflicts_with = "enable")]
        disable: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete a scheduled automation
    #[command(visible_alias = "remove", visible_alias = "rm")]
    Delete {
        automation_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Run a scheduled automation immediately
    #[command(visible_alias = "run-now")]
    Run {
        automation_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Disable a scheduled automation
    Pause {
        automation_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Enable a scheduled automation
    Resume {
        automation_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show recent automation runs
    Activity {
        automation_id: String,
        /// Number of runs to fetch
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Offset for pagination
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Automation trigger management
    Trigger {
        #[command(subcommand)]
        action: AutomationTriggerAction,
    },
}

#[derive(Subcommand)]
pub(crate) enum AutomationTriggerAction {
    /// Data-change triggers backed by the app database event stream
    Data {
        #[command(subcommand)]
        action: AutomationDataTriggerAction,
    },
}

#[derive(Subcommand)]
pub(crate) enum DbAction {
    /// Dynamic table management
    Table {
        #[command(subcommand)]
        action: DbTableAction,
    },
    /// Dynamic field management
    Field {
        #[command(subcommand)]
        action: DbFieldAction,
    },
    /// Record CRUD
    Record {
        #[command(subcommand)]
        action: DbRecordAction,
    },
    /// Run read-only SQL
    Sql {
        /// SQL query. Quote it as one argument or pass words and Garyx will join them with spaces.
        sql: Vec<String>,
        /// Maximum rows returned
        #[arg(long)]
        limit: Option<usize>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show database events
    Events {
        /// Filter by table
        #[arg(long)]
        table: Option<String>,
        /// Filter by event type: record.created, record.updated, record.deleted, schema.changed
        #[arg(long = "event-type")]
        event_type: Option<String>,
        /// Number of events to fetch
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Offset for pagination
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum DbTableAction {
    /// List dynamic tables
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create a dynamic table
    Create {
        table: String,
        /// Human display name; actual table name remains snake_case
        #[arg(long = "display-name")]
        display_name: Option<String>,
        /// Field spec in name:TYPE form. TYPE is TEXT, INTEGER, REAL, BLOB, or ANY.
        #[arg(long = "field")]
        fields: Vec<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show table schema
    Schema {
        table: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Drop a dynamic table
    Drop {
        table: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum DbFieldAction {
    /// Add a field to an existing table
    Add {
        table: String,
        field: String,
        #[arg(name = "type")]
        field_type: String,
        #[arg(long)]
        not_null: bool,
        #[arg(long)]
        unique: bool,
        #[arg(long)]
        index: bool,
        #[arg(long = "display-name")]
        display_name: Option<String>,
        /// JSON default value
        #[arg(long = "default")]
        default_value: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Drop a field from an existing table
    Drop {
        table: String,
        field: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum DbRecordAction {
    /// Insert a record from JSON object data
    Insert {
        table: String,
        /// JSON object, e.g. '{"name":"Test User"}'
        #[arg(long)]
        data: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Get a record by id
    Get {
        table: String,
        id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Update a record from JSON object data
    Update {
        table: String,
        id: String,
        /// JSON object, e.g. '{"score":10}'
        #[arg(long)]
        data: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete a record by id
    Delete {
        table: String,
        id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum AutomationDataTriggerAction {
    /// List data triggers
    List {
        /// Filter by table
        #[arg(long)]
        table: Option<String>,
        /// Filter by event type
        #[arg(long = "event-type")]
        event_type: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create a data trigger that creates a Garyx task
    Create {
        table: String,
        event_type: String,
        /// Human-readable trigger name
        #[arg(long)]
        label: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: String,
        #[arg(long = "agent-id")]
        agent_id: Option<String>,
        #[arg(long = "workspace-dir")]
        workspace_dir: Option<String>,
        #[arg(long)]
        disabled: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Enable a data trigger
    Enable {
        trigger_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Disable a data trigger
    Disable {
        trigger_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete a data trigger
    Delete {
        trigger_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum AgentAction {
    /// List all agents (built-in and custom)
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Get a custom agent
    Get {
        /// Agent id
        agent_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create a custom agent
    #[command(visible_alias = "add", arg_required_else_help = true)]
    Create {
        /// Agent id (slug, e.g. spec-review)
        #[arg(long)]
        agent_id: String,
        /// Display name
        #[arg(long, alias = "name")]
        display_name: String,
        /// Provider type: claude_code, codex_app_server, traex, gemini_cli, gpt, anthropic, or google
        #[arg(long, default_value = "claude_code")]
        provider: String,
        /// Optional model override. Omit to use the provider default.
        #[arg(long)]
        model: Option<String>,
        /// Optional reasoning effort override: low, medium, high, or xhigh.
        #[arg(long)]
        model_reasoning_effort: Option<String>,
        /// Optional model service tier override, for example priority for Fast mode.
        #[arg(long)]
        model_service_tier: Option<String>,
        /// Native model auth source, for example codex or api_key.
        #[arg(long, alias = "auth-source")]
        provider_auth_source: Option<String>,
        /// Native model API key. Stored on the custom agent provider config.
        #[arg(long, alias = "api-key")]
        provider_api_key: Option<String>,
        /// Optional default workspace directory for new task/bot threads using this agent.
        #[arg(long)]
        default_workspace_dir: Option<String>,
        /// System prompt
        #[arg(long)]
        system_prompt: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Update a custom agent
    #[command(arg_required_else_help = true)]
    Update {
        /// Agent id (slug, e.g. spec-review)
        #[arg(long)]
        agent_id: String,
        /// Display name
        #[arg(long, alias = "name")]
        display_name: String,
        /// Provider type: claude_code, codex_app_server, traex, gemini_cli, gpt, anthropic, or google
        #[arg(long, default_value = "claude_code")]
        provider: String,
        /// Optional model override. Omit to use the provider default.
        #[arg(long)]
        model: Option<String>,
        /// Optional reasoning effort override: low, medium, high, or xhigh. Pass an empty string to clear it.
        #[arg(long)]
        model_reasoning_effort: Option<String>,
        /// Optional model service tier override, for example priority for Fast mode. Pass an empty string to clear it.
        #[arg(long)]
        model_service_tier: Option<String>,
        /// Native model auth source, for example codex or api_key.
        #[arg(long, alias = "auth-source")]
        provider_auth_source: Option<String>,
        /// Native model API key. Stored on the custom agent provider config.
        #[arg(long, alias = "api-key")]
        provider_api_key: Option<String>,
        /// Optional default workspace directory for new task/bot threads using this agent. Pass an empty string to clear it.
        #[arg(long)]
        default_workspace_dir: Option<String>,
        /// System prompt
        #[arg(long)]
        system_prompt: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create or update a custom agent
    #[command(arg_required_else_help = true)]
    Upsert {
        /// Agent id (slug, e.g. spec-review)
        #[arg(long)]
        agent_id: String,
        /// Display name
        #[arg(long, alias = "name")]
        display_name: String,
        /// Provider type: claude_code, codex_app_server, traex, gemini_cli, gpt, anthropic, or google
        #[arg(long, default_value = "claude_code")]
        provider: String,
        /// Optional model override. Omit to use the provider default.
        #[arg(long)]
        model: Option<String>,
        /// Optional reasoning effort override: low, medium, high, or xhigh. Pass an empty string to clear it.
        #[arg(long)]
        model_reasoning_effort: Option<String>,
        /// Optional model service tier override, for example priority for Fast mode. Pass an empty string to clear it.
        #[arg(long)]
        model_service_tier: Option<String>,
        /// Native model auth source, for example codex or api_key.
        #[arg(long, alias = "auth-source")]
        provider_auth_source: Option<String>,
        /// Native model API key. Stored on the custom agent provider config.
        #[arg(long, alias = "api-key")]
        provider_api_key: Option<String>,
        /// Optional default workspace directory for new task/bot threads using this agent. Pass an empty string to clear it.
        #[arg(long)]
        default_workspace_dir: Option<String>,
        /// System prompt
        #[arg(long)]
        system_prompt: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete a custom agent
    Delete {
        /// Agent id
        agent_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum TeamAction {
    /// List teams
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Get one team
    Get {
        /// Team id
        team_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create a team
    Create {
        #[arg(long)]
        team_id: String,
        #[arg(long, alias = "name")]
        display_name: String,
        #[arg(long)]
        leader_agent_id: String,
        #[arg(long = "member-agent-id", required = true)]
        member_agent_ids: Vec<String>,
        #[arg(long)]
        workflow_text: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Update a team
    Update {
        team_id: String,
        #[arg(long)]
        new_team_id: Option<String>,
        #[arg(long, alias = "name")]
        display_name: String,
        #[arg(long)]
        leader_agent_id: String,
        #[arg(long = "member-agent-id", required = true)]
        member_agent_ids: Vec<String>,
        #[arg(long)]
        workflow_text: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete a team
    Delete {
        /// Team id
        team_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ThreadAction {
    /// List threads
    List {
        /// Include hidden threads
        #[arg(long)]
        include_hidden: bool,
        /// Limit
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Offset
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Get one thread
    Get {
        thread_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show a thread's message history, tool calls, and runtime records
    History {
        /// Canonical thread id
        thread_id: String,
        /// Maximum number of records/history items to fetch
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Send a message into an internal thread and stream the response
    #[command(
        override_usage = "garyx thread send <thread|task|bot> <target> [message]...",
        long_about = "Send a message into an internal Garyx thread and stream the agent response.\n\nTargets:\n  thread <thread_id>              Send to a canonical thread id\n  task <task_id>                  Resolve a task to its backing thread\n  bot <channel:account_id>        Resolve the bot's bound main thread inside the gateway\n\nExamples:\n  garyx thread send thread thread::abc \"hello\"\n  garyx thread send task '#TASK-1' \"status?\"\n  garyx thread send bot telegram:main \"continue\"\n\nFor compatibility, `garyx thread send <thread_id> [message]...` is still accepted."
    )]
    Send {
        /// Destination kind: thread, task, or bot
        kind: Option<String>,
        /// Thread id, task id, or bot selector
        target: Option<String>,
        /// Message text (reads from stdin if omitted)
        #[arg(value_name = "MESSAGE", num_args = 0..)]
        message: Vec<String>,
        /// Deprecated: use `garyx thread send bot <channel:account_id> ...`
        #[arg(long, value_name = "CHANNEL:ACCOUNT_ID", hide = true)]
        bot: Option<String>,
        /// Workspace directory for the agent
        #[arg(long)]
        workspace_dir: Option<String>,
        /// Wait up to this many seconds for a response
        #[arg(long, default_value_t = 300)]
        timeout: u64,
        /// Output raw JSON events instead of streaming text
        #[arg(long)]
        json: bool,
    },
    /// Create a thread
    Create {
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        workspace_dir: Option<String>,
        /// Create a managed git worktree for this thread. Requires workspace-dir to be a git repo root.
        #[arg(long)]
        worktree: bool,
        /// Agent or team id to bind the new thread to. Team ids and standalone
        /// agent ids share one namespace; passing a team id binds the thread to
        /// the whole team (meta-provider: `agent_team`). Omit for the default
        /// single-agent mode.
        #[arg(long)]
        agent_id: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum WorkflowAction {
    /// Manage reusable workflow definitions
    Definition {
        #[command(subcommand)]
        action: WorkflowDefinitionAction,
    },
    /// List workflow runs
    List {
        /// Restrict to a parent thread
        #[arg(long = "thread", alias = "parent-thread-id")]
        parent_thread_id: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Get a workflow run
    Get {
        workflow_run_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show workflow events
    Events {
        workflow_run_id: String,
        /// Event sequence cursor
        #[arg(long, default_value_t = 0)]
        after: u64,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Cancel a workflow run
    Cancel {
        workflow_run_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum WorkflowDefinitionAction {
    /// List workflow definitions
    List {
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long)]
        json: bool,
    },
    /// Get one workflow definition
    Get {
        workflow_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Install or update a file-backed workflow package
    Upsert {
        /// Workflow package directory or garyx.workflow.json manifest path
        #[arg(long)]
        file: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum DreamAction {
    /// List persisted dream topics for a time window
    #[command(alias = "ls")]
    List {
        /// RFC3339 lower bound. Defaults to --since-hours before now.
        #[arg(long)]
        from: Option<String>,
        /// RFC3339 upper bound. Defaults to now.
        #[arg(long)]
        to: Option<String>,
        /// Look back this many hours when --from is omitted.
        #[arg(long, default_value_t = 24)]
        since_hours: i64,
        /// Maximum topics to show
        #[arg(long, default_value_t = 80)]
        limit: usize,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Scan recent user messages and upsert dream topics in that window
    Scan {
        /// RFC3339 lower bound. Defaults to --since-hours before now.
        #[arg(long)]
        from: Option<String>,
        /// RFC3339 upper bound. Defaults to now.
        #[arg(long)]
        to: Option<String>,
        /// Look back this many hours when --from is omitted.
        #[arg(long, default_value_t = 24)]
        since_hours: i64,
        /// Extraction mode: auto, claude, or heuristic
        #[arg(long, default_value = "auto", value_parser = ["auto", "claude", "heuristic"])]
        mode: String,
        /// Maximum user messages to inspect
        #[arg(long, default_value_t = 600)]
        limit: usize,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show or set the hourly automatic dream scan switch
    Auto {
        /// Desired state: status, on, or off
        #[arg(default_value = "status", value_parser = ["status", "on", "off"])]
        state: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show one dream topic and its thread spans
    Show {
        /// Dream id
        dream_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum TaskAction {
    /// List tasks
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        source_thread: Option<String>,
        #[arg(long)]
        source_task: Option<String>,
        #[arg(long)]
        source_bot: Option<String>,
        #[arg(long)]
        include_done: bool,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long)]
        json: bool,
    },
    /// Get one task by id
    Get {
        task_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Create a new task thread
    Create {
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        start: bool,
        #[arg(long)]
        workspace_dir: Option<String>,
        /// Create the backing thread in a managed git worktree. Requires workspace-dir to be a git repo root.
        #[arg(long)]
        worktree: bool,
        /// Run this task with an agent executor
        #[arg(long, conflicts_with_all = ["team", "workflow"])]
        agent: Option<String>,
        /// Run this task with an Agent Team executor
        #[arg(long, conflicts_with_all = ["agent", "workflow"])]
        team: Option<String>,
        /// Run this task with a reusable workflow definition instead of an agent
        #[arg(long, conflicts_with_all = ["agent", "team"])]
        workflow: Option<String>,
        /// Plain-text input passed to the workflow entrypoint
        #[arg(long, requires = "workflow", conflicts_with_all = ["input_file", "input_json"])]
        input: Option<String>,
        /// Read plain-text input for the workflow entrypoint from a file
        #[arg(long, requires = "workflow", conflicts_with_all = ["input", "input_json"])]
        input_file: Option<PathBuf>,
        /// JSON input passed to the workflow entrypoint
        #[arg(long, requires = "workflow", conflicts_with_all = ["input", "input_file"])]
        input_json: Option<String>,
        /// Required notification target when the task enters review: `none`, `current-thread`, `thread <thread_id>`, or `bot <channel:account_id>`
        #[arg(long, value_name = "TARGET", num_args = 1..=2)]
        notify: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Claim a task
    Claim {
        task_id: String,
        #[arg(long)]
        actor: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Release a task
    Release {
        task_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Stop a running task run and release the task
    Stop {
        task_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Delete task metadata while retaining the backing thread transcript
    Delete {
        task_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Assign a task
    Assign {
        task_id: String,
        principal: String,
        #[arg(long)]
        json: bool,
    },
    /// Clear task assignee
    Unassign {
        task_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Update task status
    Update {
        task_id: String,
        #[arg(long)]
        status: String,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        json: bool,
    },
    /// Reopen a done task
    Reopen {
        task_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Set task title
    SetTitle {
        task_id: String,
        title: String,
        #[arg(long)]
        json: bool,
    },
    /// Show task history
    History {
        task_id: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum PluginsAction {
    /// Install a subprocess channel plugin from a binary path.
    ///
    /// garyx will run the plugin's own `initialize` + `describe`
    /// handshake to discover its id, version, capabilities, and
    /// schema, then generate a `plugin.toml` and copy the binary
    /// into `~/.garyx/plugins/<id>/`. No hand-editing required.
    Install {
        /// Path to the plugin binary.
        path: PathBuf,
        /// Install root override. Defaults to `~/.garyx/plugins/`.
        #[arg(long)]
        target: Option<PathBuf>,
        /// Overwrite an existing installation without prompting.
        #[arg(long)]
        force: bool,
    },
    /// List installed subprocess channel plugins.
    List {
        /// Install root override. Defaults to `~/.garyx/plugins/`.
        #[arg(long)]
        target: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Uninstall a subprocess channel plugin by id.
    Uninstall {
        /// Plugin id to remove.
        id: String,
        /// Install root override. Defaults to `~/.garyx/plugins/`.
        #[arg(long)]
        target: Option<PathBuf>,
    },
    // Architecture C: `garyx plugins update` was retired together with
    // the host-driven update loop. Plugins now own their upgrade timer
    // and call the `request_self_replace` host RPC when they decide
    // to swap; operators publish a new release on the plugin's update
    // server and the plugin's next tick picks it up. To force an
    // immediate replace, re-`install` the plugin from a local binary.
}
