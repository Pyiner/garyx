# Installation

Garyx ships as a single CLI binary. The recommended path on macOS and Linux
is the [Homebrew tap](https://github.com/Pyiner/homebrew-garyx); a shell
installer and a `cargo build` path are also supported.

## Install the CLI

::: code-group

```bash [Homebrew]
brew tap pyiner/garyx
brew install pyiner/garyx/garyx
```

```bash [Shell installer]
curl -fsSL https://raw.githubusercontent.com/Pyiner/garyx/main/install.sh | bash
```

```bash [From source]
git clone https://github.com/Pyiner/garyx
cd garyx
cargo build --release
# binary: target/release/garyx
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

For day-to-day use, install it as a managed background service:

```bash
garyx gateway install   # writes the launchd plist (macOS) or systemd unit (Linux)
                        # and starts it; safe to re-run after config changes
garyx gateway restart   # pick up new config
garyx gateway stop      # stop the managed service
```

For one-off testing, run it in the foreground:

```bash
garyx gateway run
```

Logs land in `~/.garyx/logs/{stdout,stderr}.log`.

::: info
On macOS the launchd plist re-enters your login shell with `sh -c "exec
$LOGIN_SHELL -lic …"`, so anything you `export` in `~/.zshenv`,
`~/.zprofile`, or `~/.zshrc` is visible to the gateway. That is the
recommended place for provider tokens like `CLAUDE_CODE_OAUTH_TOKEN`.
:::

## Verify

```bash
curl -s http://127.0.0.1:31337/health
garyx status
garyx doctor
```

Send a message into a fresh thread end-to-end:

```bash
TID=$(garyx thread create --workspace-dir "$PWD" --json | jq -r .thread_id)
garyx thread send thread "$TID" "What does this workspace do?"
```

If that round-trips, the gateway is healthy and at least one provider is
reachable.

## Where to go next

- [Your first bot](/first-bot) — wire up Telegram, Feishu, or WeChat
- [Providers](/concepts/providers) — log in to Claude Code, Codex, or Gemini
- [Service manager](/reference/service-manager) — what `gateway install`
  actually writes
