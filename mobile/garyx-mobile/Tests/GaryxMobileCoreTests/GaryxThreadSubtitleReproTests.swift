import Foundation
import XCTest
@testable import GaryxMobileCore

/// Deterministic red reproductions for #TASK-2571. They are opt-in so the
/// intentionally failing desired-contract assertions do not poison normal CI:
///
///     TASK_2571_REPRO=1 swift test --filter GaryxThreadSubtitleReproTests
@MainActor
final class GaryxThreadSubtitleReproTests: XCTestCase {
    func testNewPrivateWorkspaceThreadDisplaysThreadIdInsteadOfAcceptedUserMessage() throws {
        try requireTask2571ReproMode()
        let capture = try loadTask2571Capture()
        let active = try XCTUnwrap(capture.activeRecent.threads.first)
        let completed = try XCTUnwrap(capture.completedRecent.threads.first)

        XCTAssertEqual(active.lastMessagePreview, "")
        XCTAssertEqual(completed.lastMessagePreview, "Latest user sentence")

        let activeSubtitle = subtitle(for: active)
        let completedSubtitle = subtitle(for: completed)
        XCTAssertEqual(
            activeSubtitle,
            "thread--00000000-0000-4000-8000-000000002571"
        )
        XCTAssertEqual(
            completedSubtitle,
            "thread--00000000-0000-4000-8000-000000002571 · Latest user sentence"
        )

        // Desired contract: the accepted user sentence, rather than the
        // basename of the implicit private workspace, owns the subtitle.
        // This assertion is intentionally red.
        XCTAssertEqual(activeSubtitle, "Latest user sentence")
    }

    func testSharedSummaryCacheMakesSubtitleJumpBetweenTwoGatewaySources() throws {
        try requireTask2571ReproMode()
        let capture = try loadTask2571Capture()
        let recent = try XCTUnwrap(capture.completedRecent.threads.first)
        let summary = try XCTUnwrap(capture.completedSummaries.threads.first)
        let cache = GaryxThreadSummaryCache()

        cache.writeThrough([recent])
        let afterRecent = try XCTUnwrap(cache.summary(for: recent.id))
        cache.writeThrough([summary])
        let afterSummary = try XCTUnwrap(cache.summary(for: recent.id))
        cache.writeThrough([recent])
        let afterNextRecent = try XCTUnwrap(cache.summary(for: recent.id))

        let observedSubtitles = [afterRecent, afterSummary, afterNextRecent].map(subtitle)
        XCTAssertEqual(
            observedSubtitles,
            [
                "thread--00000000-0000-4000-8000-000000002571 · Latest user sentence",
                "thread--00000000-0000-4000-8000-000000002571 · Assistant answer",
                "thread--00000000-0000-4000-8000-000000002571 · Latest user sentence",
            ]
        )

        // Desired contract: arrival order of two list sources must not change
        // the row's visible subtitle. This assertion is intentionally red.
        XCTAssertEqual(observedSubtitles, Array(repeating: observedSubtitles[0], count: 3))
    }

    private func subtitle(for thread: GaryxThreadSummary) -> String? {
        GaryxSidebarThreadRowPresentation(
            thread: thread,
            isSelected: false,
            isPinned: false,
            trailingTimestamp: nil
        ).subtitle
    }
}

private struct Task2571ThreadSubtitleCapture: Decodable {
    let activeRecent: GaryxRecentThreadsPage
    let activeSummaries: GaryxThreadSummariesPage
    let completedRecent: GaryxRecentThreadsPage
    let completedSummaries: GaryxThreadSummariesPage

    enum CodingKeys: String, CodingKey {
        case activeRecent = "active_recent"
        case activeSummaries = "active_summaries"
        case completedRecent = "completed_recent"
        case completedSummaries = "completed_summaries"
    }
}

private func requireTask2571ReproMode() throws {
    guard ProcessInfo.processInfo.environment["TASK_2571_REPRO"] == "1" else {
        throw XCTSkip("Set TASK_2571_REPRO=1 to run the intentional red reproduction")
    }
}

private func loadTask2571Capture() throws -> Task2571ThreadSubtitleCapture {
    let url = try XCTUnwrap(
        Bundle.module.url(
            forResource: "task-2571-thread-subtitle-capture",
            withExtension: "json",
            subdirectory: "Fixtures"
        )
    )
    return try JSONDecoder().decode(
        Task2571ThreadSubtitleCapture.self,
        from: Data(contentsOf: url)
    )
}
