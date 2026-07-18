import XCTest
@testable import GaryxMobileCore

final class GaryxComposerPayloadDirectoryTests: XCTestCase {
    private let g1 = GaryxGatewayScope(identity: "g1", epoch: 1)
    private let g2 = GaryxGatewayScope(identity: "g2", epoch: 1)

    func testConversationPanelConversationRestoresTextAndAttachmentForSameKey() throws {
        var directory = GaryxComposerPayloadDirectory()
        XCTAssertEqual(
            directory.activate(
                scope: g1,
                key: .thread("A"),
                creating: entryID("a"),
                generation: 1,
                lifecycleNonce: "nonce-a"
            ),
            .created(entryID("a"))
        )
        XCTAssertTrue(directory.updateActiveText("draft A", generation: 1))
        XCTAssertTrue(directory.addActiveAttachment(attachment("a-file", generation: 1)))

        directory.suspendPresentation()
        XCTAssertEqual(
            directory.activate(
                scope: g1,
                key: .thread("A"),
                creating: entryID("unused"),
                generation: 99,
                lifecycleNonce: "unused"
            ),
            .restored(entryID("a"))
        )
        XCTAssertEqual(directory.activeProjection?.text, "draft A")
        XCTAssertEqual(directory.activeProjection?.attachments.map(\.id), [attachmentID("a-file")])
    }

    func testTwoOccurrencesOfSameConversationShareOneStableEntry() {
        var directory = GaryxComposerPayloadDirectory()
        XCTAssertEqual(
            directory.activate(
                scope: g1,
                key: .thread("A"),
                creating: entryID("a"),
                generation: 1,
                lifecycleNonce: "nonce-a"
            ),
            .created(entryID("a"))
        )
        XCTAssertTrue(directory.updateActiveText("from first occurrence", generation: 1))
        directory.suspendPresentation()

        XCTAssertEqual(
            directory.activate(
                scope: g1,
                key: .thread("A"),
                creating: entryID("second-occurrence-must-not-own-payload"),
                generation: 2,
                lifecycleNonce: "unused"
            ),
            .restored(entryID("a"))
        )
        XCTAssertEqual(directory.activeProjection?.entryID, entryID("a"))
        XCTAssertEqual(directory.activeProjection?.text, "from first occurrence")
    }

    func testEmptyTextRetainsKeyAndAttachmentInsteadOfDeletingEntry() {
        var directory = GaryxComposerPayloadDirectory()
        _ = directory.activate(
            scope: g1,
            key: .draft("new"),
            creating: entryID("draft"),
            generation: 7,
            lifecycleNonce: "draft-nonce"
        )
        XCTAssertTrue(directory.updateActiveText("temporary", generation: 7))
        XCTAssertTrue(directory.addActiveAttachment(attachment("photo", generation: 7)))
        XCTAssertTrue(directory.updateActiveText("", generation: 7))
        directory.suspendPresentation()

        XCTAssertEqual(
            directory.activate(
                scope: g1,
                key: .draft("new"),
                creating: entryID("replacement"),
                generation: 8,
                lifecycleNonce: "replacement"
            ),
            .restored(entryID("draft"))
        )
        XCTAssertEqual(directory.activeProjection?.text, "")
        XCTAssertEqual(directory.activeProjection?.attachments.map(\.id), [attachmentID("photo")])
    }

    func testGatewaySwitchSuspendsPartitionAndSwitchBackRestoresPayload() {
        var directory = GaryxComposerPayloadDirectory()
        _ = directory.activate(
            scope: g1,
            key: .draft("new"),
            creating: entryID("g1-entry"),
            generation: 1,
            lifecycleNonce: "g1-nonce"
        )
        XCTAssertTrue(directory.updateActiveText("G1 draft", generation: 1))
        XCTAssertTrue(directory.addActiveAttachment(attachment("g1-file", generation: 1)))

        _ = directory.activate(
            scope: g2,
            key: .draft("new"),
            creating: entryID("g2-entry"),
            generation: 1,
            lifecycleNonce: "g2-nonce"
        )
        XCTAssertTrue(directory.updateActiveText("G2 draft", generation: 1))
        XCTAssertEqual(directory.activeProjection?.attachments, [])

        XCTAssertEqual(
            directory.activate(
                scope: g1,
                key: .draft("new"),
                creating: entryID("unused"),
                generation: 2,
                lifecycleNonce: "unused"
            ),
            .restored(entryID("g1-entry"))
        )
        XCTAssertEqual(directory.activeProjection?.text, "G1 draft")
        XCTAssertEqual(directory.activeProjection?.attachments.map(\.id), [attachmentID("g1-file")])
    }

    func testRequestActivationTokenRejectsPreSwitchCompletionAfterScopeReactivation() {
        let first = GaryxGatewayRequestToken(scope: g1, activationSequence: 1)
        let g2Activation = GaryxGatewayRequestToken(scope: g2, activationSequence: 2)
        let restored = GaryxGatewayRequestToken(scope: g1, activationSequence: 3)

        XCTAssertNotEqual(first, g2Activation)
        XCTAssertNotEqual(first, restored)
        XCTAssertEqual(first.scope, restored.scope)
    }

    private func entryID(_ value: String) -> GaryxComposerPayloadEntryID {
        GaryxComposerPayloadEntryID(rawValue: value)
    }

    private func attachmentID(_ value: String) -> GaryxAttachmentID {
        GaryxAttachmentID(rawValue: value)
    }

    private func attachment(_ value: String, generation: UInt64) -> GaryxComposerAttachment {
        GaryxComposerAttachment(
            id: attachmentID(value),
            stagedAssetID: GaryxStagedAssetID(rawValue: "asset-\(value)"),
            generation: generation,
            byteCount: 10,
            kind: "file",
            name: "\(value).txt",
            mediaType: "text/plain"
        )
    }
}
