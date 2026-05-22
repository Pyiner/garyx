# Gary X Mobile

`GaryxMobile.xcodeproj` builds the Gary X iOS app. It uses the existing Garyx
gateway directly, with no account binding or separate mobile backend in this
app.

On a real phone, connect to the Mac's LAN address and paste the gateway token:

```text
Gateway URL: http://192.168.1.20:31337
Gateway Token: output from `garyx gateway token`
```

`127.0.0.1` is only useful in the iOS simulator because it points back to the
Mac running the simulator.

The Mac app can also generate a `garyx://mobile/connect?...` QR/link from its
Desktop Settings view. Scanning or opening that link imports `gatewayUrl` and
`gatewayAuthToken` into iOS. The token is stored in the iOS Keychain; provider
API keys remain on the gateway host and are not copied to the phone.

The package currently covers:

- Gateway URL normalization and token handling.
- `GET /api/status`, `GET /api/chat/health`, `GET /api/threads`,
  `POST /api/threads`, `PATCH /api/threads/:id`, and
  `DELETE /api/threads/:id`.
- `GET /api/thread-pins` plus `PUT`/`DELETE /api/thread-pins/:id` for shared
  gateway-backed pinned threads.
- `GET /api/threads/history`, `GET /api/threads/:id/logs`, and
  `POST /api/chat/interrupt`.
- `GET /api/custom-agents`, `GET /api/custom-agents/models`, generated avatar
  helper support, `GET /api/teams`, custom agent/team create, update, and
  delete helpers.
- `GET /api/skills` plus create, update, toggle, delete, tree, file read/save,
  and entry create/delete helpers.
- `GET /api/tasks`, `POST /api/tasks`, promote, assign/unassign, title/status
  updates, stop, and delete.
- `GET /api/automations`, create, update, delete, activity, run-now, and
  pause/enable.
- Workspace file browsing and preview via `/api/workspace-files`, plus
  workspace git capability checks, workspace file upload, and chat attachment
  upload.
- Slash command shortcuts via `/api/commands/shortcuts`.
- MCP server list/create/update/delete/toggle via `/api/mcp-servers`.
- Auto Research run list/create/detail/iterations/candidates/select/stop/delete
  helpers.
- Channel plugin catalog/auth/validation helpers, channel endpoints,
  configured bots, bot status/bind/unbind, and bot console summaries.
- `/api/chat/ws` WebSocket URL construction.
- Chat WebSocket command encoding for `start`, `input`, `recover`, and
  `interrupt`, including prompt attachments.
- Chat WebSocket event decoding for the stream event types used by the Garyx
  desktop client.

The app shell uses an iOS drawer: chat stays as the primary surface, and the
left sidebar owns conversation search plus entry points for tasks, automation
runs, files, agents, skills, commands, MCP, research, bots, and settings.
Provider credentials, model keys, local provider homes, and gateway-side runtime
configuration remain gateway-owned; mobile reuses the LAN gateway token and
sends the same REST/WebSocket operations the Mac app uses.

See [docs/mac-parity-plan.md](docs/mac-parity-plan.md) for the mobile parity
checklist and the current ownership split between iOS, the Mac app, and the
gateway.

Generate the Xcode project and run the package tests with:

```bash
cd mobile/garyx-mobile
xcodegen generate
swift test
```
