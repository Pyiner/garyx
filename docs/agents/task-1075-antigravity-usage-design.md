# TASK-1075 Antigravity Usage Design

## Goal

Extend the local coding-usage surface so the Mac and iOS model-provider pages
show Claude Code, Codex, and Antigravity usage. Antigravity usage is per model:
each upstream quota bucket renders as one model row. The iOS widget remains a
two-provider widget and shows only Claude Code plus Codex.

## Gateway Contract

`GET /api/usage/coding` remains the single endpoint. Existing Claude Code and
Codex fields stay unchanged. Additive Antigravity data is carried by an
optional per-provider model bucket field:

```json
{
  "providers": [
    {
      "id": "antigravity",
      "name": "Antigravity",
      "available": true,
      "models": [
        {
          "id": "claude-opus-4-6-thinking",
          "name": "Claude Opus 4.6 (Thinking)",
          "remaining_fraction": 0.99,
          "remaining_percent": 99,
          "used_percent": 1,
          "resets_at": "2030-01-01T00:00:00Z",
          "reset_after_seconds": 3600,
          "description": "Quota resets in 1 hour."
        }
      ]
    }
  ],
  "refreshed_at": "2030-01-01T00:00:00Z"
}
```

Claude Code and Codex keep using `weekly` and `session` windows. They do not
serialize `models`, preserving existing widget decoding and older clients.

The upstream quota payload shape is:

```json
{
  "groups": [
    {
      "buckets": [
        {
          "bucketId": "claude-opus-4-6-thinking",
          "displayName": "Claude Opus 4.6 (Thinking)",
          "resetTime": "2030-01-01T00:00:00Z",
          "description": "Quota resets in 1 hour.",
          "remainingFraction": 0.99
        }
      ]
    }
  ]
}
```

Parser mapping:

- `bucketId` -> `models[].id`
- `displayName` -> `models[].name`
- `remainingFraction` -> `remaining_fraction`, `remaining_percent`, and
  `used_percent`
- `resetTime` -> `resets_at` and derived `reset_after_seconds`
- `description` -> `description`

Parser tests must use structurally accurate synthetic values only.

## Antigravity Auth

The durable Antigravity credential source on macOS is the Keychain item:

```bash
security find-generic-password -s gemini -a antigravity -w
```

The value is `go-keyring-base64:` plus base64-encoded JSON shaped like:

```json
{
  "token": {
    "access_token": "...",
    "refresh_token": "...",
    "token_type": "Bearer",
    "expiry": "2030-01-01T00:00:00+08:00"
  },
  "auth_method": "consumer"
}
```

The local file under `~/.gemini/antigravity-cli/` can be stale and must not be
used as the durable source.

Token resolution:

1. Read and decode the Keychain item.
2. Use `access_token` when `expiry` is still valid with a small skew.
3. If expired, try Google OAuth refresh with the Antigravity installed-app
   client id. The client id is public and can be committed; real tokens cannot.
4. If direct refresh fails, run the configured Antigravity CLI with `models`
   once with a timeout, then reread Keychain. Antigravity itself refreshes this
   Keychain item.
5. Return an unavailable provider with a sanitized error if no valid token can
   be obtained.

The CLI refresh fallback is allowed only with guardrails:

- Resolve the binary from the same Antigravity config used by the provider
  runtime (`antigravity_bin`, default `agy`), not a hardcoded path.
- Limit CLI refresh attempts with a process-wide minimum interval and cache the
  unavailable result so polling clients do not spawn the CLI every 20 seconds.
- Wrap the whole Antigravity provider branch in an independent hard timeout so
  it cannot make Claude Code and Codex usage responses wait on a slow subprocess.

## Project Discovery

Use the valid Antigravity bearer token to call:

```text
POST https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist
```

Read `cloudaicompanionProject` from the response and use it as the body
`project` for:

```text
POST https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary
```

The local Antigravity project cache contains workspace project identifiers and
is not the primary quota-project source.

Live smoke on 2026-06-22 confirmed, with values redacted:

- `loadCodeAssist`: HTTP 200, `cloudaicompanionProject` present.
- `retrieveUserQuotaSummary`: HTTP 200, 8 model buckets returned.

## UI Plan

Usage provider ids are not the same as all provider/config ids. Every provider
row must carry an explicit `usageProviderId` and tests must assert the Codex
case:

| Provider row | provider type | config key | usage provider id |
| --- | --- | --- | --- |
| Claude Code | `claude_code` | `claude` | `claude_code` |
| Codex | `codex_app_server` | `codex` | `codex` |
| Antigravity | `antigravity` | `antigravity` | `antigravity` |

Mac:

- Add Antigravity to the model-provider table with provider type
  `antigravity`, built-in agent/config key `antigravity`, and usage provider
  id `antigravity`.
- Add `usageProviderId` to the fixed provider row type. Do not infer usage by
  `providerType` or `agentId`; that would miss Codex.
- Add a desktop IPC/API getter for `/api/usage/coding`.
- Render a Usage column. Claude Code and Codex show weekly remaining quota.
  Antigravity shows compact per-model rows from `models`.
- Add a desktop test or focused renderer assertion that the Codex row maps
  `codex_app_server` to usage id `codex`.

iOS:

- Extend `GaryxMobileCore` usage models with per-model buckets.
- Add provider-page usage presentation helpers in Core.
- Add Antigravity to `GaryxModelProviderDefaults` and provider identity
  presentation.
- Add `usageProviderId` to `GaryxModelProviderDefault`, with Codex explicitly
  mapped to `codex`.
- Store fetched coding usage in `GaryxMobileModel` state as well as the widget
  snapshot.
- Render usage on the model-provider page as native grouped rows.
- Add SwiftPM tests for `GaryxModelProviderDefaults.provider(for:)` and Codex
  usage lookup through `usageProviderId`.

Widget:

- Keep rendering only provider ids `claude_code` and `codex`.
- Move provider filtering into `GaryxMobileCore`, for example
  `GaryxUsageGaugeModel.widgetModels(from:now:)`, so the no-Antigravity widget
  contract is SwiftPM-testable.
- The Widget SwiftUI view must call the Core helper and must not keep a private
  fallback-to-all implementation.

## Validation

- Gateway unit tests for Keychain payload decode, project response parsing,
  quota bucket parsing, and unavailable/error paths.
- Live ignored gateway smoke for Antigravity with redacted output.
- SwiftPM tests for Core decoding, provider-page usage presentation,
  Antigravity defaults/identity, and widget provider filtering.
- iOS simulator build after mobile changes.
- Desktop UI build.
- Post-build endpoint smoke verifying three providers and Antigravity bucket
  count without printing credentials or project ids.
