# MCP integration

[Model Context Protocol](https://modelcontextprotocol.io/) is the standard
way Garyx-managed agents call back into the gateway. Every thread gets its
own per-thread MCP HTTP endpoint, so a tool call from inside an agent run
is automatically scoped to the right thread, channel, and run id.

## The shape

When Garyx spawns a provider CLI for a run, it injects an MCP config like
this:

```json
{
  "mcpServers": {
    "garyx": {
      "type": "http",
      "url": "http://127.0.0.1:31337/mcp/<thread_id>/<run_id>",
      "headers": {
        "X-Run-Id":      "<run_id>",
        "X-Thread-Id":   "<thread_id>",
        "X-Session-Key": "<thread_id>"
      }
    }
  }
}
```

The provider sees a single `garyx` MCP server. The URL is unique per
(thread, run), so any tool call can be authoritatively traced back without
trusting client-supplied identifiers.

## What lives behind the endpoint

Behind `/mcp/<thread>/<run>` Garyx exposes tools for:

- **Cross-channel messaging** — send a message to another bot or thread,
  optionally with images / files.
- **Thread management** — fork a new thread, rebind the current endpoint to
  a new agent, list recent threads.
- **Channel control** — pause / resume polling, lookup endpoint metadata.
- **Persistent storage** — wiki / knowledge-base CRUD when a wiki is
  attached.
- **Outbound search** — when a search backend is configured under
  `gateway.search`.

The exact tool set evolves; the source of truth is the gateway's
[MCP module](https://github.com/Pyiner/garyx/tree/main/garyx-gateway/src/mcp).

## Configuring upstream MCP servers

Garyx can also act as a **client** to your own MCP servers — useful when you
want every agent to inherit the same tool set (search, browser automation,
internal APIs). Add them under `mcp_servers` in `garyx.json`:

```json
{
  "mcp_servers": {
    "browser": {
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-browser"]
    },
    "internal-search": {
      "transport": "streamable_http",
      "url": "https://mcp.example.com",
      "bearer_token_env": "EXAMPLE_MCP_TOKEN"
    }
  }
}
```

These are merged into every provider's MCP config alongside the built-in
`garyx` server.

## Where to go next

- [Configuration](/configuration) — full schema for `mcp_servers`
- [Channels → the `api` channel](/concepts/channels#the-api-channel) — the
  HTTP / WebSocket surface MCP tools call back into
- [Architecture: command list design](/architecture/command-list-design) —
  how slash commands and MCP tools coexist
