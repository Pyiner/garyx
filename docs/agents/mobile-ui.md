# Mobile UI Rules

- The Mac app is the source of truth for mobile information architecture,
  labels, field meaning, icon semantics, and Gateway-backed data models.
- Mobile may adapt layout and interaction for iOS, but must not invent new
  top-level concepts.
- Use native iOS patterns for management surfaces: grouped lists, compact rows,
  top navigation actions, segmented controls for peer categories, and row-level
  ellipsis menus for secondary actions.
- Do not port desktop card/action-bar layouts directly into mobile.
- Build settings and creation flows from SwiftUI `Form`, `Section`,
  `LabeledContent`, `Picker`, `Toggle`, and system toolbar placements before
  introducing custom form geometry.
- Keep short values and controls in adaptive labeled rows. Give prompts,
  descriptions, environment variables, and other long or structured values a
  vertical row or focused editor instead of compressing them into a trailing
  column.
- Do not rely on `TextField` or `SecureField` placeholders as the only field
  label inside grouped forms.
- A grouped form section is already the content surface. Do not place editable
  fields in additional filled input wells or recreate section cards, separators,
  and row insets by hand.
- Use native cancellation, confirmation, and Done actions in the navigation
  toolbar. Keep destructive actions explicit and separate from Save.
- Keep ordinary form actions, switches, pickers, and selection monochrome.
  Green is semantic status color, not the default interactive tint.
- Use Garyx's existing adaptive glass/material helpers for mobile chrome.
- Bottom composers and attached trays should read as native iOS material:
  layered glass, system tint, fine highlights, and subtle shadow, not flat gray
  slabs.
- Keep content readable, near-white, and integrated; reserve glass for
  navigation and transient controls rather than repeated content rows.
- Treat Dynamic Type, dark mode, increased contrast, Reduce Motion, and Reduce
  Transparency as layout inputs. Do not assume a fixed label column or a
  light-only palette.
- Mobile top-left controls and leading-edge gestures must share the same route
  action.
- The home root is the pinned+recent thread list; conversations and module
  panels push above it in a navigation stack and pop back to home. Bot,
  workspace, and automation-thread drilldowns go back to whatever opened them
  (home or the originating page), never to an overview list.
- Mobile entry points that open an existing thread by row tap, widget link,
  task, automation, bot conversation, or deep link should route through the
  shared `GaryxMobileModel.openThread` path; home-list behavior is the
  baseline.
- Mobile existing-thread opens should keep transcript loading automatic,
  including cold-start retry after transient gateway failures.
- Do not surface a manual Reload button for the initial empty-message state.
- Mobile Agent/Team management `Chat` actions should open a new-thread draft
  with a one-off target override, matching the Mac app.
- Do not mutate the saved default selected agent or eagerly create an empty
  thread; `Use` owns default agent selection.
- The navigation drawer shows Automation and Agents entries with Bots and
  Workspaces expanded inline as flat children, the gateway identity control as
  the drawer header, and a floating Settings pill at the drawer bottom; Skills
  live under Settings. A thread-mode automation's
  triggered threads open from that automation's row actions.
- Mobile has no Tasks management surface: the conversation task-tree sidebar
  (right-edge swipe on a thread page) is the only task display. Do not re-add
  a task list panel, task detail/create forms, or a View-tasks thread menu
  item.
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
- Mobile chat transcript rows, tool groups, tail thinking, and active tool
  state must render from server `render_state`. Keep
  `GaryxMobileRenderStateMapper` as a dumb mapper from snapshot refs to local
  message bodies; do not re-add Swift user-turn grouping, tool pairing, or
  tail-thinking derivation.
- Mobile chat, transcript, automation, widget, and workspace/bot visual details
  live in the `garyx-product-ui` skill. Use that skill for non-trivial mobile UI
  implementation or review.
- Pure mobile route state, presentation mapping, formatting, and business-rule
  transformations should live under
  `mobile/garyx-mobile/Sources/GaryxMobileCore` with SwiftPM tests.
- Keep app-target files focused on SwiftUI composition, bindings, platform
  adapters, and side-effect orchestration.
- Mobile low-frequency catalog data such as agents, teams, workspaces, bots,
  skills, automations, slash commands, and MCP servers should use
  gateway-scoped stale-while-refresh caching.
- Restored rows are display projections only; edit paths that preserve hidden
  gateway fields must fetch authoritative data before saving.
- Keep mobile SwiftUI feature surfaces in feature-specific files rather than
  adding large view trees to `GaryxMobileViews.swift`.
