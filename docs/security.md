# Security and privacy

Garyx is local-first: the gateway, config, transcripts, channel accounts, and
desktop settings live on machines you control. That makes deployment simple,
but it does not make Garyx a sandbox.

## Runtime boundaries

- Provider CLIs run with the permissions of the gateway process.
- `workspace_dir` is an execution context, not a security boundary. If a
  provider has shell access, it can access anything the gateway user can
  access.
- The managed gateway serves APIs, WebSockets, MCP, and health endpoints. It
  does not serve a browser dashboard; use the macOS app or CLI for interactive
  operation.
- Protected gateway APIs require the configured gateway auth token. `/health`
  remains public so service managers and monitors can check liveness.
- Channel providers receive whatever text, files, images, and metadata their
  integrations require to deliver messages.

Use OS-level users, filesystem permissions, containers, or VMs when you need
hard isolation between projects or credentials.

## Secrets

Keep secrets in environment variables, not in committed config files:

```json
{
  "channels": {
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
```

Garyx expands `${VAR}` and `${VAR:-default}` when it loads `garyx.json`.

Common sensitive values:

- Telegram bot tokens
- Feishu / Lark app ids and app secrets
- WeChat / Weixin tokens, uins, and context tokens
- Gateway auth tokens
- Provider OAuth tokens and API keys
- MCP bearer tokens
- Real chat ids, user ids, bot ids, and endpoint binding keys
- Personal local paths such as a real home directory or private repository path

## Logs

By default, `garyx logs tail` reads the managed gateway stderr log. Runtime
warnings, provider errors, and channel delivery failures are written there.

Before sharing logs:

- redact tokens, authorization headers, provider credentials, and app secrets
- replace real chat ids and user ids with placeholders
- replace private file paths with paths such as `/path/to/repo`
- remove message text that contains private or customer data

Useful commands:

```bash
garyx logs path
garyx logs tail --lines 200
garyx doctor
garyx status
```

## Public examples and tests

This repository is public. Use synthetic placeholders in docs, tests, issues,
and commit messages.

Use examples like:

```text
Test User
thread::<id>
telegram:main
TOKEN_FROM_BOTFATHER
${TELEGRAM_BOT_TOKEN}
/path/to/repo
bot@example.test
```

Do not use:

- real names
- real Telegram, WeChat, Feishu, or Lark chat ids
- real user ids or bot ids
- real email addresses or phone numbers
- real home-directory paths
- provider OAuth strings
- channel tokens or API keys

## Gateway exposure checklist

If you expose Garyx beyond localhost:

1. Set a gateway auth token with `garyx gateway token`.
2. Prefer a reverse proxy with TLS.
3. Bind only the interface you intend to expose.
4. Keep provider and channel credentials in the gateway user's environment.
5. Review which folders are used as thread workspaces.
6. Keep logs local or redact them before shipping to external log systems.

## Where to go next

- [Installation](/installation) - managed service setup and verification
- [Configuration](/configuration) - gateway auth, channels, providers, MCP
- [Service manager](/reference/service-manager) - launchd / systemd behavior
