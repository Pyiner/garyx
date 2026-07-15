import Observation
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxHomeObservationBridgeTests: XCTestCase {
    func testConversationWritesDoNotInvalidateStaticHomeStoreReadsButHomeWritesDo() {
        let model = makeModel()
        let thread = makeThread(id: "thread-home-observation")
        model.selectedThread = thread
        let store = model.homeObservationStore

        var conversationInvalidations = 0
        trackStaticHomeReads(store) {
            conversationInvalidations += 1
        }

        model.setRenderSnapshot(
            GaryxRenderSnapshot(
                basedOnSeq: 1,
                rows: [],
                tailActivity: .thinking
            ),
            for: thread.id
        )
        model.setMessages([
            GaryxMobileMessage(
                id: "message-1",
                role: .assistant,
                text: "streaming",
                isStreaming: true
            )
        ], for: thread.id)

        XCTAssertEqual(conversationInvalidations, 0)

        var homeInvalidations = 0
        trackStaticHomeReads(store) {
            homeInvalidations += 1
        }

        // Drive a home pagination write through the real path: priming the
        // pager flips hasMoreThreadSummaries/footer state, and the pager's
        // didSet republishes the observation-store pagination snapshot.
        var feeds = model.recentThreadFeeds
        let ticket = feeds.requestRefresh(filter: .all)!
        feeds.completeRefresh(
            ticket,
            pageIds: [],
            pageOffset: 0,
            pageCount: 30,
            hasMore: true
        )
        model.recentThreadFeeds = feeds

        XCTAssertEqual(homeInvalidations, 1)
    }

    func testHomeThreadListStorePublishesFromActorSnapshotsWithoutLegacyDerivation() async throws {
        try XCTSkipIf(
            !HomeProjectionLiveSourceConfiguration.usesActorSnapshots,
            "Actor cutover bridge assertions are not meaningful while the rollback env flag is disabled."
        )
        let model = makeModel()
        let thread = makeThread(id: "thread-actor-home")

        model.threads = [thread]
        primeRecentFeed(model, ids: [thread.id])
        await model.homeProjectionGateway.waitForIdleForTesting()

        XCTAssertEqual(model.homeThreadListStore.snapshot.sections.allRows.map(\.id), [thread.id])
        XCTAssertEqual(model.homeThreadListStore.acceptedInputCount, 0)
        XCTAssertGreaterThan(model.homeThreadListStore.acceptedActorSnapshotCount, 0)
        XCTAssertEqual(
            model.homeThreadListStore.sectionDerivationCount,
            0,
            "Actor-backed live rendering must not derive home sections in the legacy main-actor store."
        )
    }

    func testCommittedRunStateDeltaDoesNotAlsoEmitFullCaptureFromDictionaryDidSet() async throws {
        try XCTSkipIf(
            !HomeProjectionLiveSourceConfiguration.usesActorSnapshots,
            "Actor cutover bridge assertions are not meaningful while the rollback env flag is disabled."
        )
        let model = makeModel()
        let thread = makeThread(id: "thread-committed-delta")

        model.threads = [thread]
        primeRecentFeed(model, ids: [thread.id])
        await model.homeProjectionGateway.waitForIdleForTesting()
        let baselineEmitCount = model.homeProjectionGateway.snapshotEmitCount

        model.applyTranscriptRunState(
            GaryxTranscriptRunState(busy: true, activeRunId: "run-committed-delta", activity: .thinking),
            threadId: thread.id
        )
        await model.homeProjectionGateway.waitForIdleForTesting()

        XCTAssertEqual(model.homeProjectionGateway.snapshotEmitCount, baselineEmitCount + 1)
        let row = try XCTUnwrap(model.homeThreadListStore.snapshot.sections.allRows.first { $0.id == thread.id })
        XCTAssertTrue(row.presentation.isRunning)
        XCTAssertEqual(model.homeThreadListStore.acceptedInputCount, 0)
        XCTAssertEqual(model.homeThreadListStore.sectionDerivationCount, 0)
    }

    private func makeModel() -> GaryxMobileModel {
        let suiteName = "GaryxHomeObservationBridgeTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set("http://127.0.0.1:31337", forKey: GaryxMobileSettingsKeys.gatewayUrl)
        return GaryxMobileModel(defaults: defaults)
    }

    private func makeThread(id: String) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: id,
            title: "Observation Thread",
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

    private func primeRecentFeed(_ model: GaryxMobileModel, ids: [String]) {
        var feeds = model.recentThreadFeeds
        let ticket = feeds.requestRefresh(filter: .all)!
        feeds.completeRefresh(
            ticket,
            pageIds: ids,
            pageOffset: 0,
            pageCount: ids.count,
            hasMore: false
        )
        model.recentThreadFeeds = feeds
    }

    private func trackStaticHomeReads(
        _ store: GaryxHomeObservationStore,
        onChange: @escaping () -> Void
    ) {
        withObservationTracking {
            _ = store.isGatewayConfigured
            _ = store.connectionState
            _ = store.debugShowsGatewaySwitcher
            _ = store.showsSettings
            _ = store.lastError
            _ = store.isLoadingMoreThreads
            _ = store.hasMoreThreadSummaries
        } onChange: {
            onChange()
        }
    }
}
