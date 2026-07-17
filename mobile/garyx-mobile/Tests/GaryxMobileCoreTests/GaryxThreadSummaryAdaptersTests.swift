import XCTest
@testable import GaryxMobileCore

final class GaryxThreadSummaryAdaptersTests: XCTestCase {
    func testCapturedNewSummaryRoutePayloadUsesStrictSnakeCaseAdapter() throws {
        let row = try decode(
            GaryxThreadSummaryRowDTO.self,
            #"""
            {
              "thread_id":"thread::summary",
              "title":"Summary",
              "workspace_dir":"/workspace/project",
              "thread_type":"chat",
              "provider_type":"codex",
              "agent_id":"reviewer",
              "created_at":"2026-07-17T00:00:00Z",
              "updated_at":"2026-07-17T01:00:00Z",
              "message_count":3,
              "last_user_message":"user",
              "last_assistant_message":"assistant",
              "last_message_preview":"preview",
              "recent_run_id":"run-old",
              "active_run_id":null,
              "worktree":{"worktree_dir":"/workspace/project/.worktrees/review"},
              "excluded_from_recent":true
            }
            """#
        )
        let summary = GaryxThreadSummaryAdapter.summary(row)
        XCTAssertEqual(summary.id, "thread::summary")
        XCTAssertEqual(summary.title, "Summary")
        XCTAssertEqual(summary.workspacePath, "/workspace/project")
        XCTAssertEqual(summary.lastMessagePreview, "preview")
        XCTAssertEqual(summary.worktreePath, "/workspace/project/.worktrees/review")
        XCTAssertTrue(summary.excludeFromRecent)
    }

    func testCapturedLegacyPointReadPreservesLabelAndNestedRuntime() throws {
        let record = try decode(
            GaryxLegacyThreadRecordDTO.self,
            #"""
            {
              "thread_id":"thread::legacy",
              "label":"Legacy label",
              "workspace_dir":"/workspace/legacy",
              "metadata":{"exclude_from_recent":" YES "},
              "history":{"recent_committed_run_ids":["run-1","run-2"],"messages":[{},{}]},
              "thread_runtime":{"agent_id":"codex","provider_type":"openai"}
            }
            """#
        )
        let summary = GaryxThreadSummaryAdapter.summary(record)
        XCTAssertEqual(summary.id, "thread::legacy")
        XCTAssertEqual(summary.title, "Legacy label")
        XCTAssertEqual(summary.messageCount, 2)
        XCTAssertEqual(summary.recentRunId, "run-2")
        XCTAssertEqual(summary.threadRuntime?.agentId, "codex")
        XCTAssertFalse(summary.excludeFromRecent)
    }

    func testCapturedAutomationCamelPayloadUsesIndependentAdapter() throws {
        let row = try decode(
            GaryxAutomationThreadSummaryDTO.self,
            #"""
            {
              "id":"thread::automation",
              "threadId":"thread::automation",
              "threadType":"chat",
              "title":"Generated run",
              "workspaceDir":"/workspace/automation",
              "agentId":"claude",
              "providerType":"anthropic",
              "messageCount":4,
              "lastAssistantMessage":"done",
              "automationId":"automation::daily",
              "automationThreadMode":"generated_thread"
            }
            """#
        )
        let summary = GaryxThreadSummaryAdapter.summary(row)
        XCTAssertEqual(summary.id, "thread::automation")
        XCTAssertEqual(summary.lastMessagePreview, "done")
        XCTAssertEqual(summary.automationId, "automation::daily")
        XCTAssertEqual(summary.automationThreadMode, "generated_thread")
        XCTAssertFalse(summary.excludeFromRecent)
    }

    func testLegacyExclusionFixtureMatrixIsSemanticallyNeutralized() throws {
        let fixtures: [(String, Bool)] = [
            (#"{}"#, false),
            (#"{"exclude_from_recent":true}"#, false),
            (#"{"exclude_from_recent":" YES "}"#, false),
            (#"{"exclude_from_recent":"1"}"#, false),
            (#"{"exclude_from_recent":"false"}"#, false),
            (#"{"exclude_from_recent":1}"#, false),
            (#"{"excludeFromRecent":true}"#, false),
            (#"{"metadata":{"exclude_from_recent":"true"}}"#, false),
            (#"{"metadata":{"excludeFromRecent":true}}"#, false),
            (#"{"automation_thread_mode":"generated_thread"}"#, false),
            (#"{"automation_thread_mode":" GENERATED_THREAD "}"#, false),
            (#"{"automationThreadMode":"generated_thread"}"#, false),
            (#"{"metadata":{"automation_thread_mode":"GENERATED_THREAD"}}"#, false),
            (#"{"metadata":{"automationThreadMode":"generated_thread"}}"#, false),
            (#"{"metadata":"not-an-object","exclude_from_recent":false}"#, false),
            (#"{"exclude_from_recent":false,"metadata":{"exclude_from_recent":"yes"}}"#, false),
            (#"{"automation_thread_mode":"ordinary","metadata":{"automation_thread_mode":"generated_thread"}}"#, false),
        ]

        for (index, fixture) in fixtures.enumerated() {
            let record = try decode(GaryxLegacyThreadRecordDTO.self, fixture.0)
            XCTAssertEqual(
                GaryxThreadSummaryAdapter.legacyRecordIsExcluded(record.payload),
                fixture.1,
                "fixture \(index): \(fixture.0)"
            )
            XCTAssertEqual(
                GaryxThreadSummaryAdapter.summary(record).excludeFromRecent,
                fixture.1,
                "adapter fixture \(index): \(fixture.0)"
            )
        }
    }

    @MainActor
    func testGeneratedLegacyPointReadCacheReplacementAllowsFavoriteAndLastOpen() throws {
        let record = try decode(
            GaryxLegacyThreadRecordDTO.self,
            #"""
            {
              "thread_id":"thread::generated-point-read",
              "label":"Generated point read",
              "automation_id":"automation::daily",
              "automation_thread_mode":"generated_thread",
              "exclude_from_recent":true,
              "metadata":{"exclude_from_recent":true}
            }
            """#
        )
        let pointReadSummary = GaryxThreadSummaryAdapter.summary(record)
        XCTAssertFalse(pointReadSummary.excludeFromRecent)
        XCTAssertEqual(pointReadSummary.automationThreadMode, "generated_thread")

        let cache = GaryxThreadSummaryCache()
        var staleSummary = pointReadSummary
        staleSummary.excludeFromRecent = true
        cache.writeThrough([staleSummary])
        cache.writeThrough([pointReadSummary])
        let summary = try XCTUnwrap(cache.summary(for: pointReadSummary.id))
        XCTAssertFalse(summary.excludeFromRecent)
        let capabilities = GaryxThreadRowCapabilityDeriver.capabilities(
            for: summary,
            context: GaryxThreadRowCapabilityContext()
        )
        XCTAssertEqual(capabilities.favorite, .addAndRemove)
        XCTAssertTrue(
            GaryxLastOpenedThreadRestorationPolicy.shouldPersistLastOpenedThread(
                excludedFromRecent: summary.excludeFromRecent
            )
        )
    }

    func testCapabilitiesFullRuleTable() {
        struct Case {
            var name: String
            var summary: GaryxThreadSummary?
            var context: GaryxThreadRowCapabilityContext
            var expected: GaryxThreadRowCapabilities
        }
        let ordinary = thread(id: "thread::ordinary")
        var excluded = ordinary
        excluded.id = "thread::excluded"
        excluded.excludeFromRecent = true
        let none = GaryxThreadRowCapabilities(
            canOpen: false,
            canPin: false,
            canArchive: false,
            favorite: .none,
            archiveStrategy: .none
        )
        let cases = [
            Case(
                name: "ordinary",
                summary: ordinary,
                context: .init(),
                expected: .init(
                    canOpen: true,
                    canPin: true,
                    canArchive: true,
                    favorite: .addAndRemove,
                    archiveStrategy: .thread
                )
            ),
            Case(
                name: "excluded-new",
                summary: excluded,
                context: .init(isFavorite: false),
                expected: .init(
                    canOpen: true,
                    canPin: true,
                    canArchive: true,
                    favorite: .none,
                    archiveStrategy: .thread
                )
            ),
            Case(
                name: "excluded-existing-favorite",
                summary: excluded,
                context: .init(isFavorite: true),
                expected: .init(
                    canOpen: true,
                    canPin: true,
                    canArchive: true,
                    favorite: .removeOnly,
                    archiveStrategy: .thread
                )
            ),
            Case(
                name: "automation-target",
                summary: ordinary,
                context: .init(automationTargetThreadIds: [ordinary.id]),
                expected: .init(
                    canOpen: true,
                    canPin: true,
                    canArchive: false,
                    favorite: .addAndRemove,
                    archiveStrategy: .none
                )
            ),
            Case(
                name: "active-run",
                summary: ordinary,
                context: .init(hasActiveRun: true),
                expected: .init(
                    canOpen: true,
                    canPin: true,
                    canArchive: false,
                    favorite: .addAndRemove,
                    archiveStrategy: .none
                )
            ),
            Case(
                name: "bot-endpoint",
                summary: ordinary,
                context: .init(botEndpointRow: true),
                expected: .init(
                    canOpen: true,
                    canPin: true,
                    canArchive: true,
                    favorite: .addAndRemove,
                    archiveStrategy: .botEndpoint
                )
            ),
            Case(
                name: "bot-endpoint-no-archive",
                summary: ordinary,
                context: .init(botEndpointRow: true, botEndpointCanArchive: false),
                expected: .init(
                    canOpen: true,
                    canPin: true,
                    canArchive: false,
                    favorite: .addAndRemove,
                    archiveStrategy: .none
                )
            ),
            Case(name: "placeholder", summary: nil, context: .init(), expected: none),
            Case(name: "not-openable", summary: ordinary, context: .init(openable: false), expected: none),
        ]

        for fixture in cases {
            XCTAssertEqual(
                GaryxThreadRowCapabilityDeriver.capabilities(
                    for: fixture.summary,
                    context: fixture.context
                ),
                fixture.expected,
                fixture.name
            )
        }
    }

    private func decode<Value: Decodable>(_ type: Value.Type, _ json: String) throws -> Value {
        try JSONDecoder().decode(type, from: Data(json.utf8))
    }

    private func thread(id: String) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: id,
            title: id,
            createdAt: nil,
            updatedAt: nil,
            lastMessagePreview: "",
            workspacePath: nil,
            messageCount: nil,
            agentId: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil
        )
    }
}
