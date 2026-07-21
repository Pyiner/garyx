# iOS Presentation Anchor Debt

Status: follow-up work; explicitly outside `#TASK-2572`.

The presentation lease owner-loss transition makes navigation recover when a
presenter disappears, but it does not make a recycled or data-owned row a good
presentation anchor. A presentation should be owned by a stable feature or
screen ancestor. Rows should emit a selection/action to that owner instead of
holding presentation state and modifiers themselves.

## Confirmed row-local call sites

The following inventory was audited while implementing the owner-loss fix on
2026-07-22. Each entry can lose its SwiftUI owner when its backing row is
removed, filtered, refreshed, or recycled while presentation UI is active.

| Surface | Row-local presentation call sites | Stable owner to introduce |
| --- | --- | --- |
| Transcript message and attachment rows | `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileMessageBubbleViews.swift:88`, `:709`, `:712`, `:787`; `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileMarkdownViews.swift:617` | Conversation/transcript host with item-based preview, selection, and share routes |
| Tool rows embedded in transcript rows | `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileToolTraceViews.swift:38`, `:347`, `:458` | Conversation/tool-group host with one gallery/call-list selection |
| Agent catalog rows | `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileAgentsViews.swift:1409`, `:1412` (`GaryxAgentCard`, created by the `ForEach` at `:20`) | `GaryxAgentsView`, using selected edit/delete agent state |
| Automation catalog rows | `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileAutomationViews.swift:156`, `:203` (`GaryxAutomationCard`, created by the `ForEach` at `:30`) | `GaryxAutomationsView`, using selected action/delete automation state |
| Configured-bot rows | `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileBotSettingsViews.swift:84`, `:87` (`GaryxConfiguredBotConfigRow`, created by the `ForEach` at `:22`) | Bot settings section owner with selected edit/delete bot state |
| Slash-command rows | `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileCommandsViews.swift:145`, `:175` (`GaryxSlashCommandCard`, created by the `ForEach` at `:44`) | `GaryxCommandsView`, using selected edit/delete command state |
| MCP server rows | `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileMcpViews.swift:208`, `:226` (`GaryxMcpServerCard`, created by the `ForEach` at `:44`) | `GaryxMcpServersView`, using selected edit/delete server state |
| Skill rows | `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileSkillsViews.swift:176`, `:192` (`GaryxSkillCard`, created by the `ForEach` at `:38`) | `GaryxSkillsView`, using selected edit/delete skill state |
| Saved-gateway rows | `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileSettingsViews.swift:346`, `:377` (`GaryxSavedGatewayProfileRow`, created by the `ForEach` at `:256`) | Gateway settings section owner with selected edit/delete profile state |

`GaryxTaskNotificationDebugFixtureView` at
`mobile/garyx-mobile/App/GaryxMobile/GaryxMobileMessageBubbleViews.swift:1053`
also owns a row-local preview, but is a DEBUG-only fixture rather than a shipped
surface.

## Reusable conditional row anchors to audit in the same follow-up

These reusable controls also attach presenters below a form/control identity
that can be conditionally replaced. They should be evaluated and migrated with
the row-local inventory, without changing their behavior as part of the lease
owner-loss fix:

- Agent target/provider/model/reasoning controls:
  `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileAgentPickerComponents.swift:498`,
  `:507`, `:571`, and
  `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileAgentsViews.swift:1558`,
  `:1616`, `:1711`.
- Generic form rows:
  `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileFormComponents.swift:402`
  and `:867`.
- Automation schedule field:
  `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileAutomationViews.swift:757`.

## Follow-up acceptance criteria

- Hoist presentation state to an always-attached feature/screen owner; data rows
  emit typed selections or actions only.
- Preserve current presentation content, result handling, and lease parentage.
- Add component tests that remove or replace the originating row while each
  presentation is active and prove the stable owner remains attached.
- Keep `GaryxPresentationLeaseSession` owner-loss recovery as a terminal safety
  property; anchor migration must not weaken or bypass it.
