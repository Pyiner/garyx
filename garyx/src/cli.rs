use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
        /// Image generation API key to persist in gateway.image_gen.api_key
        #[arg(long)]
        image_gen_api_key: Option<String>,
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
    /// Run local environment/config audit
    Audit {
        /// Output as JSON
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
    /// Runtime diagnostics for bot and thread investigations
    Debug {
        #[command(subcommand)]
        action: DebugAction,
    },
    /// Auto Research run management
    #[command(name = "auto-research", visible_alias = "autoresearch", alias = "ar")]
    AutoResearch {
        #[command(subcommand)]
        action: AutoResearchAction,
    },
    /// Custom agent management
    #[command(name = "agent", alias = "agents", visible_alias = "custom-agent")]
    Agent {
        #[command(subcommand)]
        action: AgentAction,
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
    /// Data migrations
    Migrate {
        #[command(subcommand)]
        action: MigrateAction,
    },
    /// Wiki knowledge base management
    #[command(name = "wiki")]
    Wiki {
        #[command(subcommand)]
        action: WikiAction,
    },
    /// Send a message via a bot
    #[command(alias = "send", alias = "msg")]
    Message {
        /// Bot selector: `channel:account_id`, e.g. `telegram:main`
        #[arg(short, long)]
        bot: String,
        /// Message text
        text: Vec<String>,
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
    Restart,
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
    /// Enable or disable an existing account
    Enable {
        /// Channel type: telegram | feishu | weixin | api | <plugin_id>
        channel: String,
        /// Account id
        account: String,
        /// Enabled flag
        #[arg(action = clap::ArgAction::Set)]
        enabled: bool,
    },
    /// Remove an existing account
    Remove {
        /// Channel type: telegram | feishu | weixin | api | <plugin_id>
        channel: String,
        /// Account id
        account: String,
    },
    /// Add a new account
    Add {
        /// Channel type: telegram | feishu | weixin | api | <plugin_id>
        channel: Option<String>,
        /// Account id
        account: Option<String>,
        /// Friendly display name
        #[arg(long)]
        name: Option<String>,
        /// Workspace directory
        #[arg(long)]
        workspace_dir: Option<String>,
        /// Agent or team id to bind this channel account to
        #[arg(long)]
        agent_id: Option<String>,
        /// Telegram bot token (for plugin-owned channels, prefer the
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
pub(crate) enum DebugAction {
    /// Inspect a single thread's runtime diagnostics
    Thread {
        /// Canonical thread id
        thread_id: String,
        /// Maximum number of records/history items to fetch
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Inspect a bot's recent diagnostics and problem threads
    Bot {
        /// Bot id like telegram:main
        bot_id: String,
        /// Maximum number of records/problem threads to fetch
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum AutoResearchAction {
    /// Create a new Auto Research run
    Create {
        /// Research goal — what you want to accomplish
        #[arg(long)]
        goal: String,
        /// Workspace directory to hand to the worker agent; defaults to the current directory
        #[arg(long)]
        workspace_dir: Option<String>,
        /// Maximum iterations (min: 1)
        #[arg(long, default_value_t = 3)]
        max_iterations: u32,
        /// Time budget in seconds
        #[arg(long, default_value_t = 15 * 60)]
        time_budget_secs: u64,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Get a run and its latest iteration
    Get {
        /// Auto Research run id
        run_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List all iterations for a run
    Iterations {
        /// Auto Research run id
        run_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Stop a running Auto Research run
    Stop {
        /// Auto Research run id
        run_id: String,
        /// Optional stop reason
        #[arg(long)]
        reason: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List all Auto Research runs
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List candidates for a run (sorted by score)
    Candidates {
        /// Auto Research run id
        run_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Patch a running Auto Research run (update parameters on-the-fly)
    Patch {
        /// Auto Research run id
        run_id: String,
        /// New max iterations
        #[arg(long)]
        max_iterations: Option<u32>,
        /// New time budget in seconds
        #[arg(long)]
        time_budget_secs: Option<u64>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Inject human feedback into a running run
    Feedback {
        /// Auto Research run id
        run_id: String,
        /// Feedback message for the next worker iteration
        message: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Force re-verify a candidate
    Reverify {
        /// Auto Research run id
        run_id: String,
        /// Candidate id to re-verify (e.g. c_3)
        candidate_id: String,
        /// Optional guidance for the verifier
        #[arg(long)]
        guidance: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Select a candidate as the final result
    Select {
        /// Auto Research run id
        run_id: String,
        /// Candidate id to select (e.g. c_3)
        candidate_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum AgentAction {
    /// List custom agents
    List {
        /// Include built-in agents
        #[arg(long)]
        include_builtin: bool,
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
    #[command(visible_alias = "add")]
    Create {
        /// Agent id (slug, e.g. spec-review)
        #[arg(long)]
        agent_id: String,
        /// Display name
        #[arg(long, alias = "name")]
        display_name: String,
        /// Provider type: claude_code, codex_app_server, or gemini_cli
        #[arg(long, default_value = "claude_code")]
        provider: String,
        /// Optional model override. Omit to use the provider default.
        #[arg(long)]
        model: Option<String>,
        /// System prompt
        #[arg(long)]
        system_prompt: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Update a custom agent
    Update {
        /// Agent id (slug, e.g. spec-review)
        #[arg(long)]
        agent_id: String,
        /// Display name
        #[arg(long, alias = "name")]
        display_name: String,
        /// Provider type: claude_code, codex_app_server, or gemini_cli
        #[arg(long, default_value = "claude_code")]
        provider: String,
        /// Optional model override. Omit to use the provider default.
        #[arg(long)]
        model: Option<String>,
        /// System prompt
        #[arg(long)]
        system_prompt: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create or update a custom agent
    Upsert {
        /// Agent id (slug, e.g. spec-review)
        #[arg(long)]
        agent_id: String,
        /// Display name
        #[arg(long, alias = "name")]
        display_name: String,
        /// Provider type: claude_code, codex_app_server, or gemini_cli
        #[arg(long, default_value = "claude_code")]
        provider: String,
        /// Optional model override. Omit to use the provider default.
        #[arg(long)]
        model: Option<String>,
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
    /// Send a message to a thread and stream the response
    Send {
        /// Thread id to send to
        thread_id: String,
        /// Message text (reads from stdin if omitted)
        message: Option<String>,
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
pub(crate) enum MigrateAction {
    /// Migrate inline thread messages into transcript files
    ThreadTranscripts {
        /// Optional session data directory override
        #[arg(long)]
        data_dir: Option<String>,
        /// Optional backup directory for original thread JSON records
        #[arg(long)]
        backup_dir: Option<String>,
        /// Rewrite thread records to transcript-backed history metadata
        #[arg(long)]
        rewrite_records: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum WikiAction {
    /// Initialize a new wiki knowledge base
    Init {
        /// Directory path for the wiki
        path: String,
        /// Topic or subject of the wiki
        #[arg(long)]
        topic: String,
        /// Wiki identifier (auto-generated from topic if omitted)
        #[arg(long)]
        id: Option<String>,
        /// Agent to bind (default: wiki-curator)
        #[arg(long, default_value = "wiki-curator")]
        agent: String,
    },
    /// List registered wikis
    List {
        #[arg(long)]
        json: bool,
    },
    /// Get wiki details
    Get {
        wiki_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Delete a wiki registration (does NOT delete files)
    Delete { wiki_id: String },
    /// Show wiki status (page counts, recent activity)
    Status {
        wiki_id: String,
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
}
