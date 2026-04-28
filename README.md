# Garyx

Garyx is a local-first AI gateway for running provider-backed agents from one
runtime. It connects a CLI, HTTP/WebSocket API, MCP tools, channel accounts, and
a macOS desktop app to shared thread history.

## Install

### Shell Script

```bash
curl -fsSL https://raw.githubusercontent.com/Pyiner/garyx/main/install.sh | bash
```

### Homebrew

```bash
brew tap pyiner/garyx https://github.com/Pyiner/garyx
brew install garyx
```

### From Source

```bash
git clone https://github.com/Pyiner/garyx
cd garyx
cargo build --release
```

The built binary is `target/release/garyx`.

## Quick Start

Create or edit `~/.garyx/garyx.json`:

```json
{
  "gateway": {
    "host": "127.0.0.1",
    "port": 31337
  },
  "channels": {
    "plugins": {
      "telegram": {
        "accounts": {
          "main": {
            "enabled": true,
            "agent_id": "claude",
            "config": {
              "token": "${TELEGRAM_BOT_TOKEN}"
            }
          }
        }
      }
    }
  }
}
```

Start the gateway:

```bash
garyx gateway run
```

Check that it is healthy:

```bash
curl http://127.0.0.1:31337/health
```

Create a thread and send a message:

```bash
garyx thread create --workspace-dir "$PWD" --json
garyx thread send thread::your-thread-id "What does this workspace do?"
```

## Common Commands

```bash
garyx gateway run
garyx gateway install
garyx gateway restart
garyx config show
garyx doctor
garyx status
garyx channels list
garyx channels add telegram main --token "$TELEGRAM_BOT_TOKEN"
garyx channels login weixin --account main
garyx plugins install ./path/to/garyx-plugin-acmechat
garyx logs tail
```

## Desktop App

The macOS desktop app lives in `desktop/garyx-desktop`.

```bash
cd desktop/garyx-desktop
npm install
npm run build:ui
npm run dist:dir
```

`npm run dist:dir` builds and installs `Garyx.app` into `/Applications`.

## Configuration

The main configuration file is `~/.garyx/garyx.json`.

Read [docs/CONFIGURATION.md](docs/CONFIGURATION.md) for the user-facing
configuration guide, including channels, plugins, providers, managed MCP
servers, automations, and desktop behavior.

## Development

Useful local checks:

```bash
cargo test --workspace --all-targets
cd desktop/garyx-desktop && npm run build:ui
cd desktop/garyx-desktop && npm run test:smoke
```

## Project Layout

```text
garyx/                  CLI binary and gateway runtime assembly
garyx-models/           Shared config, provider, and thread data types
garyx-core/             Pure routing, key, label, and slash-command logic
garyx-router/           Thread records, transcripts, endpoint routing
garyx-bridge/           Claude Code, Codex, Gemini, and team orchestration
garyx-channels/         Built-in channel runtimes and plugin host protocol
garyx-gateway/          HTTP API, MCP server, automations, skills, desktop API
desktop/garyx-desktop/  macOS Electron desktop app
```

## License

See [LICENSE](LICENSE).
