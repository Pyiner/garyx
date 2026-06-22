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

    func testHomeListStoreIgnoresRunMetadataChurnThatDoesNotChangeVisibleSnapshot() {
        var base = GaryxHomeListFixture.makeInputs(threadCount: 50, runningCount: 1)
        let store = GaryxHomeThreadListStore()
        XCTAssertTrue(store.apply(GaryxHomeThreadListInput(base)))
        let baselineSnapshot = store.snapshot
        XCTAssertEqual(store.sectionDerivationCount, 1)

        var publishes = 0
        let cancellable = store.objectWillChange.sink { publishes += 1 }
        defer { cancellable.cancel() }

        for index in 0..<300 {
            base.threads[0].activeRunId = "run-\(index)"
            base.threads[0].runState = index.isMultiple(of: 2) ? "running" : "running "
            XCTAssertFalse(
                store.apply(GaryxHomeThreadListInput(base)),
                "Run metadata churn with the same running row must not publish to the home list."
            )
        }

        XCTAssertEqual(store.snapshot, baselineSnapshot)
        XCTAssertEqual(publishes, 0)
        XCTAssertEqual(store.acceptedInputCount, 1)
        XCTAssertEqual(store.sectionDerivationCount, 1)
    }

    func testHomeListStorePublishesRowRunningChangeWithoutRederivingSections() throws {
        let base = GaryxHomeListFixture.makeInputs(threadCount: 50, runningCount: 0)
        let store = GaryxHomeThreadListStore()
        XCTAssertTrue(store.apply(GaryxHomeThreadListInput(base)))
        XCTAssertEqual(store.sectionDerivationCount, 1)

        var publishes = 0
        let cancellable = store.objectWillChange.sink { publishes += 1 }
        defer { cancellable.cancel() }

        var running = base
        running.busyThreadIds = ["thread-10"]
        XCTAssertTrue(store.apply(GaryxHomeThreadListInput(running)))

        let row = try XCTUnwrap(store.snapshot.sections.recent.first { $0.id == "thread-10" })
        XCTAssertTrue(row.presentation.isRunning)
        XCTAssertEqual(publishes, 1)
        XCTAssertEqual(store.acceptedInputCount, 2)
        XCTAssertEqual(
            store.sectionDerivationCount,
            1,
            "Running-only changes must reuse the section derivation and publish only the folded row snapshot."
        )
    }

    func testHomeListStoreAppliesVisibleChangesDirectlyWithoutInteractionFreeze() throws {
        let base = GaryxHomeListFixture.makeInputs(threadCount: 50, runningCount: 0)
        let store = GaryxHomeThreadListStore()
        var inputBuildCount = 0

        func makeInput(_ input: HomeThreadSectionsReference.Inputs) -> GaryxHomeThreadListInput {
            inputBuildCount += 1
            return GaryxHomeThreadListInput(input)
        }

        XCTAssertTrue(store.apply(makeInput(base)))
        XCTAssertEqual(inputBuildCount, 1)
        XCTAssertEqual(store.acceptedInputCount, 1)
        XCTAssertEqual(store.publishCount, 1)
        XCTAssertEqual(store.sectionDerivationCount, 1)

        var publishes = 0
        let cancellable = store.objectWillChange.sink { publishes += 1 }
        defer { cancellable.cancel() }

        var changed = base
        changed.threads[0].title = "Direct title update"
        changed.busyThreadIds = ["thread-0"]
        changed.selectedThreadId = "thread-0"
        XCTAssertTrue(store.apply(makeInput(changed)))

        XCTAssertEqual(inputBuildCount, 2)
        XCTAssertEqual(publishes, 1)
        XCTAssertEqual(store.publishCount, 2)
        XCTAssertEqual(store.acceptedInputCount, 2)

        let row = try XCTUnwrap(store.snapshot.sections.allRows.first { $0.id == "thread-0" })
        XCTAssertEqual(row.presentation.title, "Direct title update")
        XCTAssertTrue(row.presentation.isRunning)
        XCTAssertTrue(row.presentation.isSelected)

        XCTAssertFalse(store.apply(makeInput(changed)))
        XCTAssertEqual(inputBuildCount, 3)
        XCTAssertEqual(publishes, 1)
    }

    @MainActor
    func testRootNavigationPathStorePublishesOnlyForPathChanges() {
        let store = GaryxRootNavigationPathStore()
        var publishes = 0
        let cancellable = store.objectWillChange.sink { publishes += 1 }
        defer { cancellable.cancel() }

        var state = GaryxMobileNavigationState()
        for _ in 0..<300 {
            XCTAssertFalse(store.apply(navigationState: state))
        }
        XCTAssertEqual(publishes, 0)
        XCTAssertEqual(store.publishCount, 0)
        XCTAssertEqual(store.path, [])

        state.openConversation(source: .replace)
        XCTAssertTrue(store.apply(navigationState: state))
        XCTAssertEqual(store.path, [.conversation])
        XCTAssertEqual(publishes, 1)
        XCTAssertEqual(store.publishCount, 1)

        for _ in 0..<300 {
            XCTAssertFalse(store.apply(navigationState: state))
        }
        XCTAssertEqual(
            publishes,
            1,
            "A render-snapshot or run-state storm must not republish the root NavigationStack path."
        )
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

private extension GaryxHomeThreadListInput {
    init(
        _ input: HomeThreadSectionsReference.Inputs,
        isLoadingThreads: Bool = false,
        isHomeVisible: Bool = true
    ) {
        self.init(
            sectionsInput: GaryxHomeThreadSectionsInput(input),
            runningThreadIds: input.busyThreadIds,
            isLoadingThreads: isLoadingThreads,
            isHomeVisible: isHomeVisible
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
