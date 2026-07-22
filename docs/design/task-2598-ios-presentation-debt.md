# Task 2598 Adjacent iOS Presentation Debt

Status: follow-up work; explicitly outside `#TASK-2598`.

`#TASK-2598` removes only the Agents Detail -> Edit full-screen nesting. The
following reachable presentation stacks remain in existing code and should be
handled by separate tasks. They are inventory, not blockers for the Agents
fix.

## Remaining nested presentation paths

| Surface | Existing path | Follow-up direction |
| --- | --- | --- |
| Focused long-text editor | `GaryxFormTextAreaRow` in `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileFormComponents.swift` owns a `garyxFullScreenCover`. It is reachable below the full-screen Agent create/edit flows (`GaryxAgentFormContent`), Automation create/edit flows (`GaryxAutomationFormFields`), and Skill create flow (`GaryxCreateSkillCard`). | Give each enclosing feature's stable presentation owner a typed focused-editor route, or push the editor inside its existing native stack. Do not add presentation delays. |
| Task-notification Markdown images | `GaryxTaskNotificationFullScreenView` is itself presented full-screen from `GaryxMobileConversationViews.swift`, while embedded `GaryxMarkdownImageView` in `GaryxMobileMarkdownViews.swift` owns another full-screen image preview. | Hoist image-preview selection to the notification owner or push/present from one stable notification presentation coordinator. |
| Provider account authentication | `GaryxModelProviderDefaultsSheet` is full-screen, can present `GaryxClaudeCodeAccountsSheet` as a sheet, and that sheet/account detail owns a full-screen authentication flow in `GaryxMobileProviderSettingsViews.swift`. | Model the provider/account/auth sequence as one stable flow owner and keep modal transitions inside that owner. |

Row-local and recyclable presentation anchors are already inventoried in
`docs/design/ios-presentation-anchor-debt.md`; that broader owner-loss cleanup
also remains out of scope here.

## Explicitly excluded resolved crash

The representable teardown publication crash reproduced by
`testBuild158BackgroundSceneTeardownDoesNotPublishDuringDismantle` was fixed by
`#TASK-2587` before this task's base commit. `#TASK-2598` neither changes nor
reimplements that lifecycle fix.
