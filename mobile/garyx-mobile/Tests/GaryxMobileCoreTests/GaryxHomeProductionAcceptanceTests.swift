import Combine
import XCTest
@testable import GaryxMobileCore

final class GaryxHomeProductionAcceptanceTests: XCTestCase {
    func testHomeSectionsComeFromProductionCoreAndDoNotBakeRunState() throws {
        let referenceInput = GaryxHomeListFixture.makeInputs(threadCount: 50, runningCount: 4)
        let sections = GaryxHomeThreadSectionsBuilder.build(
            GaryxHomeThreadSectionsInput(referenceInput)
        )

        XCTAssertEqual(sections.pinned.count, 6)
        XCTAssertEqual(sections.recent.count, 44)
        XCTAssertEqual(sections.allRows.count, 50)

        let pinnedRunningRow = try XCTUnwrap(sections.pinned.first)
        XCTAssertEqual(pinnedRunningRow.id, "thread-0")
        XCTAssertEqual(pinnedRunningRow.presentation.title, "Conversation about topic number 0")
        XCTAssertEqual(pinnedRunningRow.presentation.subtitle, "project-0 · This is a representative multi-word last message preview for row 0 with enough text to exercise the compacted-preview string work.")
        XCTAssertFalse(
            pinnedRunningRow.presentation.isRunning,
            "Running state must be row-scoped, not baked into the cached section row."
        )
        XCTAssertEqual(pinnedRunningRow.timestampValue, referenceInput.threads[0].updatedAt)
        XCTAssertTrue(
            pinnedRunningRow.canArchive,
            "Busy state must not be baked into the cached section row; the row action checks live run state."
        )

        let automationThread = try XCTUnwrap(sections.recent.first { $0.id == "thread-49" })
        XCTAssertFalse(automationThread.canArchive)
    }

    func testHomeSectionCacheIgnoresRunStateDeltaStorm() {
        let base = GaryxHomeListFixture.makeInputs(threadCount: 50, runningCount: 1)
        let cache = GaryxHomeThreadSectionsCache()

        for index in 0..<300 {
            var next = base
            next.threads[0].activeRunId = "run-\(index)"
            next.threads[0].runState = index.isMultiple(of: 2) ? "running" : "running "
            _ = cache.sections(for: GaryxHomeThreadSectionsInput(next))
        }

        XCTAssertEqual(cache.derivationCount, 1)
    }

    func testHomeSectionCacheRecomputesForDisplayChangesOnly() {
        let base = GaryxHomeListFixture.makeInputs(threadCount: 50, runningCount: 1)
        let cache = GaryxHomeThreadSectionsCache()
        _ = cache.sections(for: GaryxHomeThreadSectionsInput(base))
        _ = cache.sections(for: GaryxHomeThreadSectionsInput(base))
        XCTAssertEqual(cache.derivationCount, 1)

        var renamed = base
        renamed.threads[0].title = "Renamed conversation"
        let renamedSections = cache.sections(for: GaryxHomeThreadSectionsInput(renamed))

        XCTAssertEqual(cache.derivationCount, 2)
        XCTAssertEqual(renamedSections.pinned.first?.presentation.title, "Renamed conversation")
    }

    func testCatalogAssignmentGateDoesNotPublishIdenticalCollections() {
        let base = GaryxHomeListFixture.makeInputs(threadCount: 10)
        let model = GaryxHomeCatalogPublicationProbe(
            agents: base.agents,
            teams: base.teams,
            automations: base.automations
        )
        var publishes = 0
        let cancellable = model.objectWillChange.sink { publishes += 1 }
        defer { cancellable.cancel() }

        XCTAssertFalse(model.apply(agents: base.agents))
        XCTAssertFalse(model.apply(teams: base.teams))
        XCTAssertFalse(model.apply(automations: base.automations))
        XCTAssertEqual(publishes, 0)

        var changedAgents = base.agents
        changedAgents[0].displayName = "Changed Agent"
        XCTAssertTrue(model.apply(agents: changedAgents))
        XCTAssertEqual(publishes, 1)
    }

    func testBackgroundReconcilePolicyPausesWhileThreadListIsInteracting() {
        XCTAssertFalse(
            GaryxBackgroundThreadReconcilePolicy.shouldRefreshThreads(
                isThreadListInteracting: true,
                candidateThreadIds: ["thread-1"]
            )
        )
        XCTAssertFalse(
            GaryxBackgroundThreadReconcilePolicy.shouldRefreshThreads(
                isThreadListInteracting: false,
                candidateThreadIds: []
            )
        )
        XCTAssertTrue(
            GaryxBackgroundThreadReconcilePolicy.shouldRefreshThreads(
                isThreadListInteracting: false,
                candidateThreadIds: ["thread-1"]
            )
        )
    }
}

private extension GaryxHomeThreadSectionsInput {
    init(_ input: HomeThreadSectionsReference.Inputs) {
        self.init(
            threads: input.threads,
            agents: input.agents,
            teams: input.teams,
            automations: input.automations,
            pinnedThreadIds: input.pinnedThreadIds,
            recentThreadIds: input.recentThreadIds,
            selectedThreadId: input.selectedThreadId
        )
    }
}

private final class GaryxHomeCatalogPublicationProbe: ObservableObject {
    @Published var agents: [GaryxAgentSummary]
    @Published var teams: [GaryxTeamSummary]
    @Published var automations: [GaryxAutomationSummary]

    init(
        agents: [GaryxAgentSummary],
        teams: [GaryxTeamSummary],
        automations: [GaryxAutomationSummary]
    ) {
        self.agents = agents
        self.teams = teams
        self.automations = automations
    }

    func apply(agents next: [GaryxAgentSummary]) -> Bool {
        GaryxEquatableAssignment.assignIfChanged(current: agents, next: next) { agents = $0 }
    }

    func apply(teams next: [GaryxTeamSummary]) -> Bool {
        GaryxEquatableAssignment.assignIfChanged(current: teams, next: next) { teams = $0 }
    }

    func apply(automations next: [GaryxAutomationSummary]) -> Bool {
        GaryxEquatableAssignment.assignIfChanged(current: automations, next: next) { automations = $0 }
    }
}
