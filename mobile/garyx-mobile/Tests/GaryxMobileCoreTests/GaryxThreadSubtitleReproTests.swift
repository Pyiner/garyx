import Foundation
import XCTest
@testable import GaryxMobileCore

/// Permanent regressions distilled from the deterministic #TASK-2571 route
/// capture. Rust covers the live write/route contract; these tests drive the
/// captured wire rows through the production Core decoder, cache, and subtitle
/// presenter.
@MainActor
final class GaryxThreadSubtitleReproTests: XCTestCase {
    func testNewPrivateWorkspaceSubtitleShowsAcceptedUserMessageWithoutImplicitPrefix() throws {
        let capture = try loadTask2571Capture()
        var active = try XCTUnwrap(capture.activeRecent.threads.first)
        let completed = try XCTUnwrap(capture.completedRecent.threads.first)

        XCTAssertEqual(active.lastMessagePreview, "")
        XCTAssertEqual(completed.lastMessagePreview, "Latest user sentence")
        XCTAssertNil(
            subtitle(for: active),
            "an empty preview must not reveal the implicit workspace basename"
        )

        // D2 makes this the live gateway value as soon as chat/start commits
        // the user row; D3 then renders only that preview.
        active.lastMessagePreview = "Latest user sentence"
        XCTAssertEqual(subtitle(for: active), "Latest user sentence")
        XCTAssertEqual(subtitle(for: completed), "Latest user sentence")
    }

    func testSharedSummaryCacheRejectsStaleRouteResponseWithoutSubtitleRegression() throws {
        let capture = try loadTask2571Capture()
        let recent = try XCTUnwrap(capture.completedRecent.threads.first)
        var staleSummary = try XCTUnwrap(capture.completedSummaries.threads.first)
        staleSummary.updatedAt = "2026-07-21T18:50:53.974042+00:00"
        let cache = GaryxThreadSummaryCache()

        cache.writeThrough([recent])
        let afterRecent = try XCTUnwrap(cache.summary(for: recent.id))
        cache.writeThrough([staleSummary])
        let afterSummary = try XCTUnwrap(cache.summary(for: recent.id))
        cache.writeThrough([recent])
        let afterNextRecent = try XCTUnwrap(cache.summary(for: recent.id))

        let observedSubtitles = [afterRecent, afterSummary, afterNextRecent].map(subtitle)
        XCTAssertEqual(
            observedSubtitles,
            Array(repeating: "Latest user sentence", count: 3)
        )
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
