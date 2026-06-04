# Mobile UI Rules

- The Mac app is the source of truth for mobile information architecture,
  labels, field meaning, icon semantics, and Gateway-backed data models.
- Mobile may adapt layout and interaction for iOS, but must not invent new
  top-level concepts.
- Use native iOS patterns for management surfaces: grouped lists, compact rows,
  top navigation actions, segmented controls for peer categories, and row-level
  ellipsis menus for secondary actions.
- Do not port desktop card/action-bar layouts directly into mobile.
- Mobile compact form rows must keep field names visible on the left and the
  value or control on the right.
- Do not rely on `TextField` or `SecureField` placeholders as the only field
  label inside grouped forms.
- Use Garyx's existing adaptive glass/material helpers for mobile chrome.
- Bottom composers and attached trays should read as native iOS material:
  layered glass, system tint, fine highlights, and subtle shadow, not flat gray
  slabs.
- Keep content readable, near-white, and integrated; reserve glass for
  navigation and transient controls rather than repeated content rows.
- Mobile top-left controls and leading-edge gestures must share the same route
  action.
- Direct sidebar children may open the sidebar; deeper pages go back to their
  immediate parent.
- Mobile entry points that open an existing thread by row tap, widget link,
  task, automation, bot conversation, or deep link should route through the
  shared `GaryxMobileModel.openThread` path; sidebar behavior is the baseline.
- Mobile existing-thread opens should keep transcript loading automatic,
  including cold-start retry after transient gateway failures.
- Do not surface a manual Reload button for the initial empty-message state.
- Mobile Agent/Team management `Chat` actions should open a new-thread draft
  with a one-off target override, matching the Mac app.
- Do not mutate the saved default selected agent or eagerly create an empty
  thread; `Use` owns default agent selection.
- Mobile sidebar root navigation shows Automation and Workspace & Bots; Tasks,
  Auto Research, Agents, and Skills live under Settings.
- Keep workspace and bot conversations inside drilldown layers rather than
  dumping raw sessions inline.
- Mobile widgets are static snapshots: do not use `ScrollView`; start directly
  with thread rows, render pinned rows like other rows, and use agent/team
  avatars where available.
- Mobile recent-thread widgets should opt out of WidgetKit's default content
  margins and use compact manual padding.
- Large widgets prioritize five readable rows over fitting extra rows with
  smaller typography.
- Recent-thread widget row taps must use per-row `Link` destinations only.
- Do not attach a container `.widgetURL` to the first thread because it can
  steal row taps and open the wrong conversation.
- Provider, agent, team, bot, and channel identity presentation must resolve
  through shared Core presentation helpers.
- Do not add local switch tables in views, widgets, or settings.
- Mobile chat, transcript, automation, widget, and workspace/bot visual details
  live in the `garyx-product-ui` skill. Use that skill for non-trivial mobile UI
  implementation or review.
- Pure mobile route state, presentation mapping, formatting, and business-rule
  transformations should live under
  `mobile/garyx-mobile/Sources/GaryxMobileCore` with SwiftPM tests.
- Keep app-target files focused on SwiftUI composition, bindings, platform
  adapters, and side-effect orchestration.
- Mobile low-frequency catalog data such as agents, teams, workspaces, bots,
  skills, automations, tasks, slash commands, and MCP servers should use
  gateway-scoped stale-while-refresh caching.
- Restored rows are display projections only; edit paths that preserve hidden
  gateway fields must fetch authoritative data before saving.
- Keep mobile SwiftUI feature surfaces in feature-specific files rather than
  adding large view trees to `GaryxMobileViews.swift`.
