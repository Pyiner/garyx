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
                tailActivity: .thinking,
                visibleMessageIds: ["message-1"]
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

        model.isLoadingMoreThreads = true

        XCTAssertEqual(homeInvalidations, 1)
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
            teamId: nil,
            teamName: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil
        )
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
