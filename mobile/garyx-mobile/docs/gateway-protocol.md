# Gary X Mobile Gateway Protocol

Gary X mobile talks to the managed gateway directly on the same network as the
Mac. The app uses the same `gatewayUrl` / `gatewayAuthToken` concept as the
desktop app.

| Capability | Gateway surface |
| --- | --- |
| Basic health | `GET /api/status` |
| Chat readiness | `GET /api/chat/health` |
| Thread list | `GET /api/threads?limit=...&offset=...` |
| Thread creation | `POST /api/threads` |
| Thread metadata | `GET /api/threads/{thread_id}` |
| Thread transcript | `GET /api/threads/history?thread_id=...` |
| Streaming chat | `GET /api/chat/ws` WebSocket |
| Stop active run | WebSocket `interrupt`, with `POST /api/chat/interrupt` as fallback |
| Agent/team selection | `GET /api/custom-agents`, `GET /api/teams` |
| Skills visibility | `GET /api/skills` |
| Task control | `GET /api/tasks`, `POST /api/tasks`, task status/stop/delete endpoints |
| Automation control | `GET /api/automations`, `POST /api/automations/{id}/run-now`, `PATCH /api/automations/{id}` |

Remote mobile clients must include the gateway token. HTTP requests use:

```text
Authorization: Bearer ${GARYX_GATEWAY_TOKEN}
```

WebSocket requests pass the same token as the `token` query parameter because
`URLSessionWebSocketTask` does not give every callsite a convenient header path.

For physical devices, the gateway URL must be reachable from the phone, usually
the Mac's LAN IP such as `http://192.168.1.20:31337`. `http://127.0.0.1:31337`
only reaches the Mac from the iOS simulator.

The Mac app can hand these settings to iOS with:

```text
garyx://mobile/connect?gatewayUrl=...&gatewayAuthToken=...
```

The mobile app stores `gatewayAuthToken` in the iOS Keychain. Model provider
keys such as OpenAI, Anthropic, Claude, Codex, or Gemini credentials stay on the
Mac/gateway because all model execution still happens there.

Mobile feature parity is intentionally gateway-first. The iOS app reads the same
thread, task, automation, custom-agent, team, and Skill resources that the Mac
app uses, but presents them as compact action panels instead of copying the Mac
window one-to-one.

The chat socket accepts JSON commands:

```json
{ "op": "start", "threadId": "thread::<id>", "message": "hello", "waitForResponse": false }
{ "op": "input", "threadId": "thread::<id>", "message": "more detail" }
{ "op": "recover", "threadId": "thread::<id>", "limit": 200 }
{ "op": "interrupt", "threadId": "thread::<id>" }
```

The stream emits event objects with a `type` field such as `accepted`,
`assistant_delta`, `tool_use`, `tool_result`, `user_ack`,
`thread_title_updated`, `done`, `stream_input`, `interrupt`, `snapshot`,
`error`, and `ping`.
