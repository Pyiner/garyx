use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::config_support::default_config_path_string;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(
    name = "garyx",
    version = VERSION,
    about = "Garyx – AI chat gateway",
    after_help = "Command groups:\n  Run the gateway     gateway, status, doctor, onboard, config, logs, update, auto-update, plugins\n  Manage assets       agent, provider, channels, commands, automation, db\n  Work with context   task, thread, meeting, message, bot, usage, tool\n\nExit codes:\n  0 success · 1 error · 2 usage error · 3 gateway unreachable · 4 not found · 5 edit conflict"
)]
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
    #[command(display_order = 1, alias = "gw")]
    Gateway {
        #[command(subcommand)]
        action: GatewayAction,
    },
    /// Configuration utilities
    #[command(display_order = 5, alias = "cfg")]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Provider defaults and quota usage
    #[command(display_order = 22, name = "provider", alias = "providers")]
    Provider {
        #[command(subcommand)]
        action: ProviderAction,
    },
    /// Coding-assistant quota usage (also shown in `garyx provider list`)
    #[command(display_order = 44)]
    Usage {
        /// Optional provider filter: claude_code, codex, or antigravity
        provider: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Chat slash commands and prompt shortcuts (not CLI commands)
    #[command(
        display_order = 24,
        name = "commands",
        visible_alias = "shortcuts",
        visible_alias = "shortcut"
    )]
    CommandList {
        #[command(subcommand)]
        action: CommandAction,
    },
    /// Show running status
    #[command(display_order = 2, alias = "ps")]
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Run health checks
    #[command(display_order = 3, alias = "check")]
    Doctor {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Guided setup for a new config
    #[command(display_order = 4, alias = "setup")]
    Onboard {
        /// Overwrite existing config file
        #[arg(long)]
        force: bool,
        /// API account id to create or enable for gateway/API usage
        #[arg(long, default_value = "main")]
        api_account: String,
        /// Start the gateway after onboarding completes
        #[arg(long)]
        run_gateway: bool,
        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },
    /// Download and replace the current garyx binary from GitHub Releases
    #[command(display_order = 7, visible_alias = "upgrade")]
    Update {
        /// Specific version to install (defaults to latest release)
        #[arg(long)]
        version: Option<String>,
        /// Override the target binary path (defaults to the current executable)
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Manage the gateway + plugin auto-update kill switches
    #[command(display_order = 8, name = "auto-update")]
    AutoUpdate {
        #[command(subcommand)]
        action: AutoUpdateAction,
    },
    /// Channel account management
    #[command(display_order = 23, alias = "channel")]
    Channels {
        #[command(subcommand)]
        action: ChannelsAction,
    },
    /// Subprocess channel plugin management
    #[command(display_order = 9, alias = "plugin")]
    Plugins {
        #[command(subcommand)]
        action: PluginsAction,
    },
    /// Local log file utilities
    #[command(display_order = 6, alias = "log")]
    Logs {
        #[command(subcommand)]
        action: LogsAction,
    },
    /// Bot status and current binding utilities
    #[command(display_order = 43, name = "bot")]
    Bot {
        #[command(subcommand)]
        action: BotAction,
    },
    /// Automation management
    #[command(display_order = 25, name = "automation")]
    Automation {
        #[command(subcommand)]
        action: AutomationAction,
    },
    /// Custom agent management
    #[command(
        display_order = 20,
        name = "agent",
        alias = "agents",
        visible_alias = "custom-agent"
    )]
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Provider-backed utility commands
    #[command(display_order = 45, name = "tool", visible_alias = "tools")]
    Tool {
        #[command(subcommand)]
        action: ToolAction,
    },
    /// Thread utilities
    #[command(display_order = 41, alias = "threads")]
    Thread {
        #[command(subcommand)]
        action: ThreadAction,
    },
    /// Task overlay utilities
    #[command(display_order = 40, alias = "tasks")]
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Read and manage captured meeting entities
    #[command(display_order = 39, alias = "meetings")]
    Meeting {
        #[command(subcommand)]
        action: MeetingAction,
    },
    /// Send an outbound channel message via a bot
    #[command(
        display_order = 42,
        alias = "send",
        alias = "msg",
        long_about = "Send an outbound channel message via a bot (to Telegram, Discord, Feishu, …).\n\nThis writes to the chat channel without running an agent. To message an internal Garyx thread and stream the agent response, use `garyx thread send`."
    )]
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
pub(crate) enum MeetingAction {
    /// List meeting entities
    List {
        /// Output the complete catalog as structured JSON
        #[arg(long)]
        json: bool,
    },
    /// Read meeting segments incrementally or as a stateless snapshot
    Read {
        /// Meeting entity UUID
        id: String,
        /// Read a stateless full snapshot
        #[arg(
            long,
            conflicts_with_all = ["range", "continue_token", "epoch", "thread"]
        )]
        full: bool,
        /// Read a stateless closed sequence range in A..B form
        #[arg(
            long,
            value_name = "A..B",
            conflicts_with_all = ["full", "continue_token", "thread"]
        )]
        range: Option<String>,
        /// Require this log epoch for a range read
        #[arg(long, requires = "range", conflicts_with = "continue_token")]
        epoch: Option<i64>,
        /// Resume a full or range snapshot using its opaque token
        #[arg(
            long = "continue",
            value_name = "TOKEN",
            conflicts_with_all = ["full", "range", "epoch", "thread"]
        )]
        continue_token: Option<String>,
        /// Reader identity override for incremental mode; defaults to GARYX_THREAD_ID
        #[arg(
            long,
            value_name = "ID",
            conflicts_with_all = ["full", "range", "continue_token"]
        )]
        thread: Option<String>,
        /// Emit each structured response page as one NDJSON line
        #[arg(long)]
        json: bool,
        /// Total structured JSON response-byte target; minimum 4096
        #[arg(long, value_name = "N")]
        max_bytes: Option<usize>,
    },
    /// Abort a joining or live meeting entity
    Abort {
        /// Meeting entity UUID
        id: String,
    },
    /// Permanently delete a terminal meeting entity
    Delete {
        /// Meeting entity UUID
        id: String,
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
        long_about = "Restart the managed gateway service and refresh its unit/plist first.\n\nEvery thread that was actively running when the gateway went down is resumed with a structured restart notice (wake-all), so an agent that restarts the gateway is continued automatically."
    )]
    Restart,
    /// Stop the managed gateway service
    Stop,
    /// Reload the running gateway config from disk without restart
    ReloadConfig,
    /// Rotate the persistent store identity after restoring or cloning a full
    /// data directory. The gateway must be stopped.
    RotateStoreIncarnation,
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
        /// Provider type: claude_code, codex_app_server, traex, antigravity, or grok_build
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
pub(crate) enum ProviderAction {
    /// List provider defaults and quota summaries
    #[command(visible_alias = "ls")]
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show one provider default configuration
    #[command(visible_alias = "get")]
    Show {
        /// Provider type: claude_code, codex_app_server, traex, antigravity, or grok_build
        provider: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Edit one provider's default configuration. Fields you omit keep their current values.
    #[command(visible_alias = "update")]
    Set {
        /// Provider type: claude_code, codex_app_server, traex, antigravity, or grok_build
        provider: String,
        /// Default model id. Omit to leave unchanged.
        #[arg(long, conflicts_with = "clear_model")]
        model: Option<String>,
        /// Clear the configured default model.
        #[arg(long)]
        clear_model: bool,
        /// Default reasoning effort / thinking level. Omit to leave unchanged.
        #[arg(long, conflicts_with = "clear_reasoning")]
        reasoning: Option<String>,
        /// Clear the configured default reasoning effort.
        #[arg(long)]
        clear_reasoning: bool,
        /// Default service tier. Omit to leave unchanged.
        #[arg(long)]
        service_tier: Option<String>,
        /// Claude Code CLI mode. Only valid when provider is claude_code.
        #[arg(long, value_parser = ["cctty", "native"])]
        claude_cli_mode: Option<String>,
        /// Explicit Claude Code CLI path. Only valid when provider is claude_code.
        #[arg(long)]
        claude_cli_path: Option<String>,
        /// Provider env entry as KEY=VALUE. May be repeated.
        #[arg(long = "env")]
        env: Vec<String>,
        /// Clear one provider env entry by writing an empty value. May be repeated.
        #[arg(long = "clear-env")]
        clear_env: Vec<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
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
    #[command(visible_alias = "delete", visible_alias = "rm")]
    Remove {
        /// Channel type: telegram | discord | feishu | weixin | api | <plugin_id>
        channel: String,
        /// Account id
        account: String,
    },
    /// Add a new account
    #[command(visible_alias = "create")]
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
        /// Agent id to bind this channel account to
        #[arg(long = "agent", alias = "agent-id")]
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
        /// Agent id to bind this channel account to
        #[arg(long = "agent", alias = "agent-id")]
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
    /// Timezone for daily schedules. Defaults to this machine's timezone when --daily-time is used.
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
        /// Automation id
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
        /// Agent id to run
        #[arg(long = "agent", alias = "agent-id")]
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
        /// Automation id
        automation_id: String,
        /// Human-readable automation name
        #[arg(long, alias = "name")]
        label: Option<String>,
        /// Prompt text
        #[arg(long)]
        prompt: Option<String>,
        /// Agent id to run
        #[arg(long = "agent", alias = "agent-id")]
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
        /// Automation id
        automation_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Run a scheduled automation immediately
    #[command(visible_alias = "run-now")]
    Run {
        /// Automation id
        automation_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Disable a scheduled automation
    Pause {
        /// Automation id
        automation_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Enable a scheduled automation
    Resume {
        /// Automation id
        automation_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show recent automation runs
    Activity {
        /// Automation id
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
}

#[derive(Subcommand)]
pub(crate) enum AgentAction {
    /// List all agents (built-in and custom)
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Enable an agent for new bindings
    Enable {
        /// Agent id
        agent_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Disable an agent for new bindings
    Disable {
        /// Agent id
        agent_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show or set the global default agent
    Default {
        /// Agent id. Omit to show the configured and effective defaults.
        agent_id: Option<String>,
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
        /// Provider type: claude_code, codex_app_server, traex, antigravity, or grok_build
        #[arg(long, default_value = "claude_code")]
        provider: String,
        /// Optional model override. Omit to use the provider default.
        #[arg(long)]
        model: Option<String>,
        /// Optional reasoning effort override, for example low, medium, high, xhigh, max, or ultra (supported values depend on the provider and model).
        #[arg(long)]
        model_reasoning_effort: Option<String>,
        /// Optional model service tier override, for example priority for Fast mode.
        #[arg(long)]
        model_service_tier: Option<String>,
        /// Optional default workspace directory for new task/bot threads using this agent.
        #[arg(long)]
        default_workspace_dir: Option<String>,
        /// Optional system prompt. Omit to use the provider default.
        #[arg(long)]
        system_prompt: Option<String>,
        /// Set an agent environment variable as KEY=VALUE (repeatable). Merged onto existing env.
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Remove an agent environment variable by KEY (repeatable).
        #[arg(long = "unset-env", value_name = "KEY")]
        unset_env: Vec<String>,
        /// Clear all agent environment variables before applying --env.
        #[arg(long = "env-clear")]
        env_clear: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Update a custom agent. Fields you omit keep their current values.
    #[command(arg_required_else_help = true)]
    Update {
        /// Agent id (slug, e.g. spec-review)
        #[arg(long)]
        agent_id: String,
        /// Display name. Omit to keep the current value.
        #[arg(long, alias = "name")]
        display_name: Option<String>,
        /// Provider type: claude_code, codex_app_server, traex, antigravity, or grok_build. Omit to keep the current value.
        #[arg(long)]
        provider: Option<String>,
        /// Optional model override. Omit to preserve the existing value.
        #[arg(long, conflicts_with = "clear_model")]
        model: Option<String>,
        /// Clear the model override and use the provider default.
        #[arg(long)]
        clear_model: bool,
        /// Optional reasoning effort override, for example low, medium, high, xhigh, max, or ultra (supported values depend on the provider and model). Pass an empty string to clear it.
        #[arg(long)]
        model_reasoning_effort: Option<String>,
        /// Optional model service tier override, for example priority for Fast mode. Pass an empty string to clear it.
        #[arg(long)]
        model_service_tier: Option<String>,
        /// Optional default workspace directory for new task/bot threads using this agent. Pass an empty string to clear it.
        #[arg(long)]
        default_workspace_dir: Option<String>,
        /// Optional system prompt. Omit to preserve the existing value; pass an empty string to clear it.
        #[arg(long)]
        system_prompt: Option<String>,
        /// Set an agent environment variable as KEY=VALUE (repeatable). Merged onto existing env.
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Remove an agent environment variable by KEY (repeatable).
        #[arg(long = "unset-env", value_name = "KEY")]
        unset_env: Vec<String>,
        /// Clear all agent environment variables before applying --env.
        #[arg(long = "env-clear")]
        env_clear: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create or update a custom agent. On update, fields you omit keep their current values.
    #[command(arg_required_else_help = true)]
    Upsert {
        /// Agent id (slug, e.g. spec-review)
        #[arg(long)]
        agent_id: String,
        /// Display name. Required when creating; omit on update to keep the current value.
        #[arg(long, alias = "name")]
        display_name: Option<String>,
        /// Provider type: claude_code, codex_app_server, traex, antigravity, or grok_build. Omit to keep the current value (claude_code when creating).
        #[arg(long)]
        provider: Option<String>,
        /// Optional model override. Omit to preserve an existing value, or use the provider default on create.
        #[arg(long, conflicts_with = "clear_model")]
        model: Option<String>,
        /// Clear the model override and use the provider default.
        #[arg(long)]
        clear_model: bool,
        /// Optional reasoning effort override, for example low, medium, high, xhigh, max, or ultra (supported values depend on the provider and model). Pass an empty string to clear it.
        #[arg(long)]
        model_reasoning_effort: Option<String>,
        /// Optional model service tier override, for example priority for Fast mode. Pass an empty string to clear it.
        #[arg(long)]
        model_service_tier: Option<String>,
        /// Optional default workspace directory for new task/bot threads using this agent. Pass an empty string to clear it.
        #[arg(long)]
        default_workspace_dir: Option<String>,
        /// Optional system prompt. Omit to preserve an existing value, or use the provider default on create. Pass an empty string to clear it.
        #[arg(long)]
        system_prompt: Option<String>,
        /// Set an agent environment variable as KEY=VALUE (repeatable). Merged onto existing env.
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Remove an agent environment variable by KEY (repeatable).
        #[arg(long = "unset-env", value_name = "KEY")]
        unset_env: Vec<String>,
        /// Clear all agent environment variables before applying --env.
        #[arg(long = "env-clear")]
        env_clear: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete a custom agent
    #[command(visible_alias = "remove", visible_alias = "rm")]
    Delete {
        /// Agent id
        agent_id: String,
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
        /// Canonical thread id, e.g. thread::abc
        thread_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Archive a thread with replay-safe lifecycle semantics
    Archive {
        /// Canonical thread id, e.g. thread::abc
        thread_id: String,
        /// Endpoint key to detach with the archive (repeatable)
        #[arg(long = "endpoint-key")]
        endpoint_keys: Vec<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Permanently delete a thread with replay-safe lifecycle semantics
    Delete {
        /// Canonical thread id, e.g. thread::abc
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
        long_about = "Send a message into an internal Garyx thread and stream the agent response.\n\nTargets:\n  thread <thread_id>              Send to a canonical thread id\n  task <task_id>                  Resolve a task to its backing thread\n  bot <channel:account_id>        Resolve the bot's bound main thread inside the gateway\n\nExamples:\n  garyx thread send thread thread::abc \"hello\"\n  garyx thread send task '#TASK-1' \"status?\"\n  garyx thread send bot telegram:main \"continue\"\n\nFor compatibility, `garyx thread send <thread_id> [message]...` is still accepted.\nTo send an outbound chat message to a channel without running an agent, use `garyx message`."
    )]
    Send {
        /// Destination kind: thread, task, or bot
        kind: Option<String>,
        /// Thread id, task id, or bot selector
        target: Option<String>,
        /// Message text (reads from stdin if omitted)
        #[arg(value_name = "MESSAGE", num_args = 0..)]
        message: Vec<String>,
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
        /// Thread title shown in thread lists
        #[arg(long)]
        title: Option<String>,
        /// Workspace directory for the thread's agent
        #[arg(long)]
        workspace_dir: Option<String>,
        /// Create a managed git worktree for this thread. Requires workspace-dir to be a git repo root.
        #[arg(long)]
        worktree: bool,
        /// Agent id to bind the new thread to. Omit for the default agent.
        #[arg(long = "agent", alias = "agent-id")]
        agent_id: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum TaskAction {
    /// List tasks (done tasks are included by default)
    List {
        /// Filter by status: todo, in_progress, in_review, or done
        #[arg(long)]
        status: Option<String>,
        /// Filter by the thread that created the task
        #[arg(long)]
        source_thread: Option<String>,
        /// Filter by the task that created the task
        #[arg(long)]
        source_task: Option<String>,
        /// Filter by the bot that created the task, e.g. telegram:main
        #[arg(long)]
        source_bot: Option<String>,
        /// Maximum number of tasks to return
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Offset for pagination
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Get one task by id
    Get {
        /// Task id, e.g. '#TASK-1' (quote the leading #)
        task_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delegate work in a new task thread
    Create {
        /// Task title shown in task lists
        #[arg(long)]
        title: Option<String>,
        /// Task body: the work description handed to the executor
        #[arg(long)]
        body: Option<String>,
        /// Workspace directory for the backing thread; defaults to the current directory
        #[arg(long)]
        workspace_dir: Option<String>,
        /// Create the backing thread in a managed git worktree. Requires workspace-dir to be a git repo root.
        #[arg(long)]
        worktree: bool,
        /// Agent that receives the delegated task
        #[arg(long)]
        agent: Option<String>,
        /// Notification target when the task enters review. Defaults to the current thread (or `none` outside a thread). Override with `none`, `current-thread`, `thread <thread_id>`, or `bot <channel:account_id>`.
        #[arg(long, value_name = "TARGET", num_args = 1..=2)]
        notify: Vec<String>,
    },
    /// Stop a running task run and release the task
    Stop {
        /// Task id, e.g. '#TASK-1' (quote the leading #)
        task_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete task metadata while retaining the backing thread transcript
    #[command(visible_alias = "remove", visible_alias = "rm")]
    Delete {
        /// Task id, e.g. '#TASK-1' (quote the leading #)
        task_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Update task status
    Update {
        /// Task id, e.g. '#TASK-1' (quote the leading #)
        task_id: String,
        /// New status: todo, in_progress, in_review, or done
        #[arg(long)]
        status: String,
        /// Note recorded in the task history for this transition
        #[arg(long)]
        note: Option<String>,
        /// Bypass the in-review handoff guard for manual status corrections
        #[arg(long)]
        force: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Reopen a done task
    Reopen {
        /// Task id, e.g. '#TASK-1' (quote the leading #)
        task_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Set task title
    SetTitle {
        /// Task id, e.g. '#TASK-1' (quote the leading #)
        task_id: String,
        /// New task title
        title: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show task history
    History {
        /// Task id, e.g. '#TASK-1' (quote the leading #)
        task_id: String,
        /// Maximum number of history entries to return
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// Every visible subcommand carries an about string and every visible
    /// argument carries a help string. The CLI's primary consumers are agents
    /// that learn a command from `--help` alone, so an undocumented flag is a
    /// usability regression — this test turns that into a build failure.
    fn assert_command_documented(command: &clap::Command, path: &str) {
        for sub in command.get_subcommands() {
            if sub.get_name() == "help" {
                continue;
            }
            let sub_path = format!("{path} {}", sub.get_name());
            assert!(
                sub.get_about().is_some() || sub.get_long_about().is_some(),
                "subcommand `{sub_path}` is missing an about string"
            );
            assert_command_documented(sub, &sub_path);
        }
        for arg in command.get_arguments() {
            if arg.is_hide_set() {
                continue;
            }
            let id = arg.get_id().as_str();
            if id == "help" || id == "version" {
                continue;
            }
            assert!(
                arg.get_help().is_some() || arg.get_long_help().is_some(),
                "argument `{id}` of `{path}` is missing a help string"
            );
        }
    }

    #[test]
    fn every_visible_cli_argument_and_subcommand_is_documented() {
        let command = Cli::command();
        assert_command_documented(&command, "garyx");
    }

    #[test]
    fn cli_definition_is_internally_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn meeting_header_commands_parse_verbatim_and_invalid_epoch_modes_are_rejected() {
        let id = "00000000-0000-7000-8000-000000000001";
        let range = Cli::try_parse_from([
            "garyx", "meeting", "read", id, "--range", "100..200", "--epoch", "7",
        ])
        .expect("range command");
        match range.command {
            Some(Commands::Meeting {
                action:
                    MeetingAction::Read {
                        range,
                        epoch,
                        continue_token,
                        ..
                    },
            }) => {
                assert_eq!(range.as_deref(), Some("100..200"));
                assert_eq!(epoch, Some(7));
                assert!(continue_token.is_none());
            }
            _ => panic!("unexpected meeting range parse"),
        }

        let continued = Cli::try_parse_from([
            "garyx",
            "meeting",
            "read",
            id,
            "--continue",
            "shell_safe-token",
        ])
        .expect("continue command");
        match continued.command {
            Some(Commands::Meeting {
                action: MeetingAction::Read { continue_token, .. },
            }) => assert_eq!(continue_token.as_deref(), Some("shell_safe-token")),
            _ => panic!("unexpected meeting continue parse"),
        }

        assert!(
            Cli::try_parse_from(["garyx", "meeting", "read", id, "--full", "--epoch", "7"])
                .is_err()
        );
        assert!(Cli::try_parse_from(["garyx", "meeting", "read", id, "--epoch", "7"]).is_err());

        let abort = Cli::try_parse_from(["garyx", "meeting", "abort", id]).expect("abort command");
        assert!(matches!(
            abort.command,
            Some(Commands::Meeting {
                action: MeetingAction::Abort { id: parsed }
            }) if parsed == id
        ));
    }
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
        /// Output as JSON
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
