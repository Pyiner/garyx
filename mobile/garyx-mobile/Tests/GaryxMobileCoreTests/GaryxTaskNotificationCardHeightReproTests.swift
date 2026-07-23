@testable import GaryxMobileCore
import XCTest

final class GaryxTaskNotificationCardHeightReproTests: XCTestCase {
    /// Sanitized from the affected thread's canonical seq 222 / 237 records
    /// and its server-owned render_state. Notification bodies are preserved;
    /// the local username and unrelated activity/tool rows are anonymized.
    func testCapturedShortNotificationHugsTwoLinesInsteadOfReservingTen() throws {
        let capture = try loadCapture()
        let cards = try mappedCards(from: capture.frame)
        let short = try XCTUnwrap(cards["#TASK-2656"])
        let reference = try XCTUnwrap(
            capture.referenceLayout.cards.first { $0.taskId == "#TASK-2656" }
        )

        XCTAssertEqual(
            short.finalMessage,
            "On it. Let me read through all the required documents and code first."
        )
        XCTAssertEqual(reference.naturalLineCount, 2)

        let lineHeight = 1.0
        let layout = GaryxTaskNotificationOverflow.collapsedBodyLayout(
            naturalHeight: Double(reference.naturalLineCount) * lineHeight,
            clampHeight: Double(capture.referenceLayout.collapsedLineLimit) * lineHeight,
            epsilon: 0.01
        )

        XCTAssertFalse(layout.isTruncated)
        XCTAssertFalse(layout.showsExpand)
        XCTAssertEqual(
            layout.displayedHeight / lineHeight,
            Double(reference.naturalLineCount),
            "A short card must display its two natural lines, not reserve the ten-line clamp."
        )
    }

    func testCapturedLongNotificationStillClampsAndShowsExpand() throws {
        let capture = try loadCapture()
        let cards = try mappedCards(from: capture.frame)
        let long = try XCTUnwrap(cards["#TASK-2655"])
        let reference = try XCTUnwrap(
            capture.referenceLayout.cards.first { $0.taskId == "#TASK-2655" }
        )

        XCTAssertTrue(long.finalMessage.contains("Verdict: **REVISE"))
        XCTAssertGreaterThan(
            reference.naturalLineCount,
            capture.referenceLayout.collapsedLineLimit
        )

        let lineHeight = 1.0
        let layout = GaryxTaskNotificationOverflow.collapsedBodyLayout(
            naturalHeight: Double(reference.naturalLineCount) * lineHeight,
            clampHeight: Double(capture.referenceLayout.collapsedLineLimit) * lineHeight,
            epsilon: 0.01
        )

        XCTAssertTrue(layout.isTruncated)
        XCTAssertTrue(layout.showsExpand)
        XCTAssertEqual(
            layout.displayedHeight / lineHeight,
            Double(capture.referenceLayout.collapsedLineLimit)
        )
    }

    private func loadCapture() throws -> Capture {
        let url = try XCTUnwrap(
            Bundle.module.url(
                forResource: "task-2663-task-notification-card-height-frame",
                withExtension: "json",
                subdirectory: "Fixtures"
            )
        )
        return try JSONDecoder().decode(Capture.self, from: Data(contentsOf: url))
    }

    private func mappedCards(
        from frame: GaryxThreadRenderFrame
    ) throws -> [String: GaryxTaskNotification] {
        let committed = frame.events.compactMap { event -> GaryxTranscriptMessage? in
            guard event.type == "committed_message",
                  let seq = event.seq,
                  var message = event.message else {
                return nil
            }
            message.applyCommittedIndex(seq - 1)
            return message
        }
        XCTAssertEqual(frame.events.compactMap(\.seq), [222, 237])
        XCTAssertEqual(committed.map(\.index), [221, 236])
        let snapshot = try XCTUnwrap(frame.renderState)
        XCTAssertEqual(snapshot.basedOnSeq, 237)

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: GaryxMobileTranscriptMapper.mobileMessages(from: committed),
            transcriptMessages: committed
        )
        return rows.reduce(into: [:]) { cards, row in
            guard let message = row.userBlock?.message,
                  case let .taskNotification(_, notification) =
                      GaryxMobileMessagePresentation.make(for: message) else {
                return
            }
            cards[notification.taskId] = notification
        }
    }
}

private struct Capture: Decodable {
    let frame: GaryxThreadRenderFrame
    let referenceLayout: ReferenceLayout

    enum CodingKeys: String, CodingKey {
        case frame
        case referenceLayout = "reference_layout"
    }
}

private struct ReferenceLayout: Decodable {
    let collapsedLineLimit: Int
    let cards: [ReferenceCard]

    enum CodingKeys: String, CodingKey {
        case collapsedLineLimit = "collapsed_line_limit"
        case cards
    }
}

private struct ReferenceCard: Decodable {
    let taskId: String
    let naturalLineCount: Int

    enum CodingKeys: String, CodingKey {
        case taskId = "task_id"
        case naturalLineCount = "natural_line_count"
    }
}
