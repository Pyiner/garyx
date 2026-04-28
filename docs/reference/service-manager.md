# Service manager

`garyx gateway install` registers the gateway with the appropriate system
service manager and starts it. The same command is safe to re-run when you
want to refresh the unit file (e.g. after upgrading Garyx).

| Platform | Backend | Unit / plist path |
| --- | --- | --- |
| macOS | launchd (Aqua user agent) | `~/Library/LaunchAgents/com.garyx.agent.plist` |
| Linux | systemd `--user` | `~/.config/systemd/user/garyx.service` |

## What gets written

Both backends share the same idea: launch `/bin/sh -c` with a one-liner
that re-enters the user's login shell with `-lic`, then `exec` the gateway.
Re-entering the shell is what makes `~/.zshrc` (and friends) source-able,
which in turn lets provider tokens like `CLAUDE_CODE_OAUTH_TOKEN` propagate
into the spawned `claude` / `codex` / `gemini` CLIs.

Concretely, the macOS plist looks like:

```xml
<key>ProgramArguments</key>
<array>
  <string>/bin/sh</string>
  <string>-c</string>
  <string>exec "$(dscl . -read /Users/$(id -un) UserShell | awk '/^UserShell:/ {print $NF}')" -lic "exec garyx gateway run --host 0.0.0.0 --port 31337"</string>
</array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><true/>
<key>LimitLoadToSessionType</key><string>Aqua</string>
<key>StandardOutPath</key><string>/Users/<you>/.garyx/logs/stdout.log</string>
<key>StandardErrorPath</key><string>/Users/<you>/.garyx/logs/stderr.log</string>
```

The Linux unit follows the same pattern and writes its journal to systemd.

::: tip Why re-enter the shell instead of baking PATH into the unit?
If we hard-coded `/opt/homebrew/bin/garyx` (or a fixed PATH), every later
move of the binary or update to your shell rcfile would require editing
the unit. By going through `dscl` + `-lic`, the gateway always picks up
the same environment your interactive shell sees.
:::

## Putting secrets in the right place

```bash
# good — sourced by the gateway via -lic
echo 'export CLAUDE_CODE_OAUTH_TOKEN="sk-ant-oat01-…"' >> ~/.zshrc
chmod 600 ~/.zshrc
```

Avoid stuffing `EnvironmentVariables` directly into the plist; the
shell-rc path keeps your gateway in sync with what you use day-to-day in
Terminal.

## Logs

```bash
garyx logs path           # absolute path
garyx logs tail --lines 200
tail -f ~/.garyx/logs/stderr.log
```

When the gateway is misbehaving, `stderr.log` is the file to read first.
Provider-side errors (claude / codex / gemini) bubble up there with their
own warnings prefixed by `claude_provider:`, `codex_provider:`, etc.

## Removing the service

```bash
garyx gateway stop
garyx gateway uninstall
```

`uninstall` removes the unit / plist; data under `~/.garyx/` is left intact
so you can reinstall later without losing thread history.

## Where to go next

- [Installation](/installation) — full install + first-run flow
- [Providers](/concepts/providers) — provider auth that this unit feeds into
- [CLI commands](/reference/cli) — every gateway subcommand
