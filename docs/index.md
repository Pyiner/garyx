# Introduction

Garyx is a local-first AI gateway. It runs as a single CLI binary plus an
optional macOS desktop app, and brings together everything you need to put
provider-backed agents in front of real users:

- a **gateway** that owns thread history, scheduling, and per-thread MCP
- **channel bots** for Telegram, Feishu / Lark, WeChat, plus an HTTP / WebSocket
  API channel for embedding Garyx into anything else
- **providers** that route runs to Claude Code, Codex, or Gemini via their
  official CLIs — log in once, share state across every channel
- a **plugin protocol** so you can ship your own channels as subprocess
  binaries without touching Garyx itself

Everything lives in one config file (`~/.garyx/garyx.json`) and one set of
data dirs (`~/.garyx/`).

## Get started

<div class="cards">

[**Install Garyx**](/installation)
Pick up the CLI from Homebrew, the install script, or `cargo build`.

[**Your first bot**](/first-bot)
Wire up Telegram, Feishu, or WeChat in three commands.

</div>

## Understand the model

<div class="cards">

[**Threads & workspaces**](/concepts/threads-and-workspaces)
What a thread is, where it lives on disk, and how `workspace_dir` ties to
the agent run.

[**Channels**](/concepts/channels)
Built-in vs plugin channels, accounts vs bindings, and how a chat ends up
attached to a thread.

[**Providers**](/concepts/providers)
How Garyx finds Claude / Codex / Gemini, where their auth lives, and what
happens when a token expires.

[**MCP integration**](/concepts/mcp)
Each thread gets its own MCP endpoint so tool calls flow back into the
gateway automatically.

</div>

## Reference

<div class="cards">

[**Configuration**](/configuration)
The full `garyx.json` schema: gateway, channels, agents, MCP servers,
automations, desktop behavior.

[**CLI commands**](/reference/cli)
Every `garyx` subcommand at a glance, organized by what you actually do
with them.

[**Service manager**](/reference/service-manager)
What `garyx gateway install` writes to launchd / systemd, and how to
debug it.

</div>

## Talk to us

- Issues and discussion: [GitHub](https://github.com/Pyiner/garyx)
- Releases: [github.com/Pyiner/garyx/releases](https://github.com/Pyiner/garyx/releases)

<style>
.cards {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: 1rem;
  margin-top: 1rem;
}
.cards p {
  margin: 0;
  padding: 1rem 1.25rem;
  border: 1px solid var(--vp-c-divider);
  border-radius: 12px;
  background: var(--vp-c-bg-soft);
  transition: border-color 0.15s, background 0.15s;
}
.cards p:hover {
  border-color: var(--vp-c-brand-1);
}
.cards p > a:first-child {
  display: block;
  font-weight: 600;
  font-size: 1.05rem;
  margin-bottom: 0.35rem;
  text-decoration: none;
  color: var(--vp-c-brand-1);
}
.cards p > a:first-child + br {
  display: none;
}
</style>
