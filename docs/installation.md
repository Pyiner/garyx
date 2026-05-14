# Installation

Garyx ships as a single CLI binary. The shell installer downloads the latest
release, verifies its checksum, copies the binary into `~/.garyx/bin`, and
signs it on macOS. It does not initialize config or start the gateway.

## Install the CLI

::: code-group

```bash [Shell installer]
curl -fsSL https://raw.githubusercontent.com/Pyiner/garyx/main/install.sh | bash
export PATH="$HOME/.garyx/bin:$PATH"
```

```bash [Homebrew]
brew tap pyiner/garyx
brew install pyiner/garyx/garyx
```

```bash [From source]
git clone https://github.com/Pyiner/garyx
cd garyx
scripts/install-local-cli.sh
```

:::

Verify the install:

```bash
garyx --version
garyx doctor
```

## Initialize the config

`garyx.json` lives at `~/.garyx/garyx.json`. Generate a minimal one with:

```bash
garyx onboard
```

That seeds an `api` channel account, default agents, and the gateway block.

::: tip
Strings support `${VAR}` and `${VAR:-default}` env-var expansion at load
time, so secrets can stay out of the file. See [Configuration](/configuration)
for the full schema.
:::

## Run the gateway

For day-to-day use, install it as a managed background service. This command
writes the launchd plist on macOS or the systemd user unit on Linux, then
starts the gateway:

```bash
garyx gateway install
```

Use `garyx gateway restart --no-wake` after config changes when no active
thread needs to be resumed. Use `garyx gateway stop` to stop the service.

For one-off testing, run it in the foreground:

```bash
garyx gateway run
```

Runtime warnings and provider/channel errors land in
`~/.garyx/logs/stderr.log`, which is what `garyx logs tail` reads by default.

::: info
On macOS the launchd plist re-enters your login shell with `sh -c "exec
$LOGIN_SHELL -lic …"`, so anything you `export` in `~/.zshenv`,
`~/.zprofile`, or `~/.zshrc` is visible to the gateway. That is the
recommended place for provider tokens like `CLAUDE_CODE_OAUTH_TOKEN`.
:::

## Verify the gateway

```bash
curl -s http://127.0.0.1:31337/health
garyx status
garyx doctor
```

If those checks pass, the gateway is healthy. The next useful step is adding a
Telegram bot so real messages can enter Garyx through a channel.

## Where to go next

- [Your first bot](/first-bot) — wire up Telegram, Feishu, or WeChat
- [Providers](/concepts/providers) — log in to Claude Code, Codex, or Gemini
- [Security](/security) — secret handling and local runtime boundaries
- [Service manager](/reference/service-manager) — what `gateway install`
  actually writes
