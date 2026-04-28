---
layout: home

hero:
  name: Garyx
  text: Local-first AI gateway
  tagline: One runtime for CLI, HTTP/WS API, MCP tools, channel bots, and a desktop app — all sharing the same thread history.
  image:
    src: /logo.svg
    alt: Garyx
  actions:
    - theme: brand
      text: Getting Started
      link: /getting-started
    - theme: alt
      text: Configuration
      link: /configuration
    - theme: alt
      text: GitHub
      link: https://github.com/Pyiner/garyx

features:
  - icon: 🧩
    title: Provider-agnostic agents
    details: Route threads to Claude Code, Codex, Gemini, or your own subprocess providers — login once, share state everywhere.
  - icon: 💬
    title: Channels in minutes
    details: Telegram, Feishu / Lark, WeChat, plus an HTTP API channel out of the box. Add a bot with a single CLI command.
  - icon: 🛠️
    title: MCP-native
    details: Each thread exposes a per-thread MCP endpoint so tool calls flow back into Garyx automatically.
  - icon: 🖥️
    title: Desktop app
    details: A macOS Electron app for managing threads, channels, automations, and live logs.
  - icon: 🔌
    title: Pluggable channels
    details: Subprocess plugins talk a JSON-RPC protocol; install third-party channels with `garyx plugins install`.
  - icon: 🗂️
    title: Single config file
    details: Everything lives in `~/.garyx/garyx.json` with `${VAR}` env-var expansion — easy to version-control, easy to migrate.
---
