# Gary X Mobile Mac Parity

Gary X Mobile is the LAN companion for the Gary X Mac app. It does not copy
provider secrets, local provider homes, or Electron-only host capabilities to
iOS. The phone talks to the same gateway API as the Mac app with the gateway
token, and the Mac/gateway remains the owner of execution, provider keys,
workspace access, channel accounts, and MCP process management.

## Implemented Mobile Surfaces

These are the Mac app surfaces that have a gateway-equivalent mobile
implementation:

- iOS shell: left drawer, conversation search, no bottom tab.
- Chat: thread list, create/delete/rename thread, history, thread logs,
  HTTP chat commands, per-thread committed transcript streaming, gateway token
  connect links, and uploaded prompt attachments.
- Tasks: list, create, status update, title edit, assign/unassign, stop,
  delete, and source-thread task filtering.
- Automations: list, create, edit, pause/enable, delete, activity, and run now.
- Workspaces: path discovery from threads/automations/defaults, manual Mac path
  entry, gateway git-status check, directory browsing, and file preview.
- Agents and teams: list/select, provider model discovery, create, edit, and
  delete. Provider secrets remain gateway-owned.
- Skills: list, create, edit metadata, toggle, delete, editor tree, file
  read/save, and entry create/delete.
- Slash commands: list/create/edit/delete.
- MCP servers: list/create/edit/delete/toggle with stdio args, env, working
  directory, streamable HTTP URL, and headers.
- Bots and channels: channel plugin catalog, configured bot summaries, bot
  status/bind/unbind, endpoint list, endpoint bind/detach, and thread open
  affordances.
- Gateway settings and mobile handoff: token storage, URL normalization,
  connect-link import, and gateway settings helpers.

## Gateway-Owned Capabilities

- Provider API keys and account auth.
- Local provider session discovery and resume.
- MCP server process execution and external config sync.
- Workspace filesystem reads/writes on the Mac.
- Channel plugin auth flows and account validation.
- Packaging, update, restart, and launchd/codesign flows.

## Intentionally Not One-to-One On iOS

These Mac APIs are Electron or host-local features. They are not claimed as
mobile parity because they have no safe iOS equivalent without changing the
product contract:

- Electron WebContents browser runtime: tabs, overlay bounds, browser back,
  forward, reload, and external browser control.
- Desktop updater/install lifecycle and app version subscriptions.
- Desktop local workspace picker/reveal/relink/rename/remove state. Mobile can
  enter Mac workspace paths and browse them through the gateway.
- Desktop-local gateway profile bookkeeping. Mobile stores only its active
  gateway URL/token pair.
- Local memory document file helper. Agent/team execution still uses the
  gateway-owned files; mobile does not directly edit host-local memory files.
- Desktop-only deep-link subscriptions. Mobile handles its own
  `garyx://mobile/connect` links.

## Validation

Run from `mobile/garyx-mobile`:

```bash
swift test
xcodebuild -project GaryxMobile.xcodeproj -scheme GaryxMobile -destination 'generic/platform=iOS Simulator' -derivedDataPath /tmp/garyx-mobile-derived CODE_SIGNING_ALLOWED=NO build
```

The fixture data in tests is synthetic and uses placeholder paths, IDs, and
tokens.
