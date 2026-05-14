---
layout: home
hero:
  name: Garyx
  text: Local-first AI agent gateway
  tagline: Run provider-backed agents from Telegram, Feishu / Lark, WeChat, CLI, HTTP / WebSocket, MCP, automations, and the macOS app with one shared thread history.
  image:
    src: /logo.svg
    alt: Garyx logo
  actions:
    - theme: brand
      text: Get started
      link: /installation
    - theme: alt
      text: Your first bot
      link: /first-bot
    - theme: alt
      text: GitHub
      link: https://github.com/Pyiner/garyx
features:
  - title: Channel gateway
    details: Telegram, Feishu / Lark, WeChat, subprocess channel plugins, and a local API channel all share the same routing model.
  - title: Provider bridge
    details: Route threads to Claude Code, Codex, Gemini, custom agents, or teams while keeping channel setup stable.
  - title: Persistent threads
    details: Conversations keep transcript history, endpoint bindings, provider resume state, and workspace context across surfaces.
  - title: Scoped MCP
    details: Every run gets a per-thread Garyx MCP endpoint plus the upstream MCP servers configured for that gateway.
  - title: Tasks and automations
    details: Promote work into reviewable tasks or schedule recurring prompts that deliver through Garyx.
  - title: macOS desktop
    details: Use the native app for browsing threads, folders, agents, tasks, gateway settings, and live runs.
---

## The shape

Garyx is the local process that turns many entry points into one durable agent
runtime.

```text
Telegram / Feishu / WeChat / CLI / Desktop / HTTP / WebSocket
  -> Garyx gateway
  -> Threads, transcripts, endpoint bindings, tasks, automations
  -> Claude Code / Codex / Gemini / custom agents / teams
  -> Garyx MCP tools and configured upstream MCP servers
```

It is not a hosted agent platform. The gateway, config, channel accounts, and
transcripts live on the machine or server where you run Garyx.

## Up and running

::: code-group

```bash [Install]
brew tap pyiner/garyx
brew install pyiner/garyx/garyx
```

```bash [Initialize]
garyx onboard
garyx gateway install
garyx status
```

```bash [First thread]
TID=$(garyx thread create --workspace-dir "$PWD" --json | jq -r .thread_id)
garyx thread send thread "$TID" "What does this workspace do?"
```

```bash [First Telegram bot]
export TELEGRAM_BOT_TOKEN="TOKEN_FROM_BOTFATHER"
garyx channels add telegram main --token "$TELEGRAM_BOT_TOKEN" --agent-id claude
garyx gateway restart --no-wake
```

:::

## Why it exists

<div class="garyx-grid">

<div class="garyx-panel">
<strong>Meet users where they already are</strong>
<p>Keep Telegram DMs, Feishu group mentions, WeChat chats, CLI sends, desktop threads, and API calls in the same agent runtime.</p>
</div>

<div class="garyx-panel">
<strong>Keep context attached to the work</strong>
<p>Threads carry transcript history, channel bindings, provider resume tokens, and a fixed <code>workspace_dir</code>.</p>
</div>

<div class="garyx-panel">
<strong>Use provider CLIs directly</strong>
<p>Garyx spawns Claude Code, Codex, and Gemini through their local CLIs, so each provider keeps its normal auth model.</p>
</div>

<div class="garyx-panel">
<strong>Expose tools through MCP</strong>
<p>Every provider run receives a scoped Garyx MCP server and any upstream MCP servers configured in <code>garyx.json</code>.</p>
</div>

<div class="garyx-panel">
<strong>Turn conversations into operations</strong>
<p>Tasks, scheduled automations, gateway restart wakeups, logs, and channel bindings all use the same thread model.</p>
</div>

<div class="garyx-panel">
<strong>Extend without patching core</strong>
<p>Install subprocess channel plugins, update them independently, and keep built-in channels in the main binary.</p>
</div>

</div>

## Runtime boundaries

<div class="garyx-callouts">

<div>
<strong>Local-first, not sandboxed</strong>
<p>Provider CLIs run with the gateway process permissions. A workspace path is an execution context, not an isolation boundary.</p>
</div>

<div>
<strong>Secrets stay out of examples</strong>
<p>Use environment variables such as <code>TELEGRAM_BOT_TOKEN</code> and placeholders such as <code>/path/to/repo</code>. Do not publish real chat ids, bot ids, tokens, or personal paths.</p>
</div>

<div>
<strong>Gateway API, not gateway web UI</strong>
<p>The managed gateway serves APIs, WebSockets, MCP, and health endpoints. The public website is this docs site; the interactive client is the macOS app.</p>
</div>

</div>

## Documentation map

<div class="docs-map">

[**Installation**](/installation)
Install the CLI, initialize config, run the managed gateway, and verify a first thread.

[**Your first bot**](/first-bot)
Wire up Telegram, Feishu / Lark, or WeChat and bind the account to an agent.

[**Threads & workspaces**](/concepts/threads-and-workspaces)
Understand thread identity, endpoint bindings, immutable workspace directories, and provider sessions.

[**Channels**](/concepts/channels)
See how built-in and plugin channels share account config and endpoint binding behavior.

[**Providers**](/concepts/providers)
Authenticate Claude Code, Codex, and Gemini, then let Garyx auto-detect them.

[**MCP integration**](/concepts/mcp)
Use Garyx MCP tools and merge in upstream MCP servers for every provider run.

[**Configuration**](/configuration)
Full `garyx.json` reference for gateway, channels, providers, agents, plugins, automations, and desktop behavior.

[**CLI commands**](/reference/cli)
Every supported command group, organized by workflow.

[**Security**](/security)
Secret handling, log redaction, local runtime boundaries, and public contribution hygiene.

</div>

<style>
.garyx-grid,
.garyx-callouts,
.docs-map {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
  gap: 16px;
  margin-top: 16px;
}

.garyx-panel,
.garyx-callouts > div,
.docs-map > p {
  border: 1px solid var(--vp-c-divider);
  border-radius: 8px;
  padding: 16px;
  background: var(--vp-c-bg-soft);
}

.garyx-panel strong,
.garyx-callouts strong {
  display: block;
  margin-bottom: 8px;
}

.garyx-panel p,
.garyx-callouts p,
.docs-map p {
  margin: 0;
}

.docs-map > p > a:first-child {
  display: block;
  font-weight: 600;
  margin-bottom: 6px;
  color: var(--vp-c-brand-1);
  text-decoration: none;
}

.docs-map > p > a:first-child + br {
  display: none;
}
</style>
