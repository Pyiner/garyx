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
| Start chat run | `POST /api/chat/start` |
| Queue follow-up input | `POST /api/chat/stream-input` |
| Live chat events | `GET /api/stream` Server-Sent Events |
| Stop active run | `POST /api/chat/interrupt` |
| Agent/team selection | `GET /api/custom-agents`, `GET /api/teams` |
| Skills visibility | `GET /api/skills` |
| Task control | `GET /api/tasks`, `POST /api/tasks`, task status/stop/delete endpoints |
| Automation control | `GET /api/automations`, `POST /api/automations/{id}/run-now`, `PATCH /api/automations/{id}` |

Remote mobile clients must include the gateway token. HTTP requests use:

```text
Authorization: Bearer ${GARYX_GATEWAY_TOKEN}
```

Server-Sent Event requests use the same `Authorization` header as other HTTP
requests.

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

Chat commands use HTTP request bodies:

```text
POST /api/chat/start
{ "threadId": "thread::<id>", "message": "hello", "waitForResponse": false }

POST /api/chat/stream-input
{ "threadId": "thread::<id>", "clientIntentId": "mobile-...", "message": "more detail" }

POST /api/chat/interrupt
{ "threadId": "thread::<id>" }
```

`GET /api/stream` emits live run events with a `type` field such as
`accepted`, `assistant_delta`, `tool_use`, `tool_result`, `user_ack`,
`thread_title_updated`, `done`, `snapshot`, `error`, and `ping`. Command
statuses for `stream_input` and `interrupt` are returned by their HTTP
endpoints; the mobile decoder still accepts those event shapes for compatibility.
