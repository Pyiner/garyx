import XCTest
@testable import GaryxMobileCore

final class GaryxTranscriptCacheTests: XCTestCase {
    private func msg(_ index: Int, _ role: GaryxTranscriptRole, _ text: String) -> GaryxTranscriptMessage {
        GaryxTranscriptMessage(index: index, role: role, text: text)
    }

    private func pageInfo(
        hasMoreBefore: Bool = false,
        nextBeforeIndex: Int? = nil,
        hasMoreAfter: Bool = false,
        nextAfterIndex: Int? = nil
    ) -> GaryxThreadTranscriptPageInfo {
        GaryxThreadTranscriptPageInfo(
            returnedMessages: 0,
            returnedStartIndex: nil,
            returnedEndIndex: nil,
            hasMoreBefore: hasMoreBefore,
            nextBeforeIndex: nextBeforeIndex,
            hasMoreAfter: hasMoreAfter,
            nextAfterIndex: nextAfterIndex
        )
    }

    // MARK: - GaryxTranscriptMessage Codable round-trip

    func testTranscriptMessageCodableRoundTripPreservesFieldsAndDerivesId() throws {
        let original = GaryxTranscriptMessage(
            index: 7,
            role: .toolUse,
            kind: "tool_use",
            text: "echo hi",
            content: .object(["command": .string("echo hi")]),
            input: .object(["tool_calls": .array([.object(["id": .string("call-cache")])])]),
            result: .object(["tool_use_id": .string("call-cache")]),
            timestamp: "2026-06-14T00:00:00Z",
            toolRelated: true,
            likelyUserVisible: false
        )
        let data = try JSONEncoder().encode(original)
        let decoded = try JSONDecoder().decode(GaryxTranscriptMessage.self, from: data)
        XCTAssertEqual(decoded, original)
        XCTAssertEqual(decoded.id, "history:7", "id re-derived from index on decode")
        XCTAssertEqual(decoded.role, .toolUse)
        XCTAssertEqual(decoded.content, .object(["command": .string("echo hi")]))
        XCTAssertEqual(decoded.input, .object(["tool_calls": .array([.object(["id": .string("call-cache")])])]))
        XCTAssertEqual(decoded.result, .object(["tool_use_id": .string("call-cache")]))
    }

    func testCachedTranscriptCodableRoundTrip() throws {
        let snapshot = GaryxCachedTranscript(
            threadId: "thread::abc",
            savedAt: Date(timeIntervalSince1970: 1_000_000),
            messages: [msg(0, .user, "hi"), msg(1, .assistant, "yo")],
            hasMoreBefore: true,
            nextBeforeIndex: 0
        )
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let decoded = try decoder.decode(GaryxCachedTranscript.self, from: encoder.encode(snapshot))
        XCTAssertEqual(decoded, snapshot)
        XCTAssertEqual(decoded.afterCursor, 1)
        XCTAssertEqual(decoded.firstIndex, 0)
    }

    // MARK: - merge logic

    func testReplaceLatestUsesFetchedWindowAndPageBeforeInfo() {
        let merged = GaryxTranscriptCacheLogic.merged(
            into: nil,
            threadId: "t",
            fetched: [msg(3, .user, "u"), msg(4, .assistant, "a")],
            pageInfo: pageInfo(hasMoreBefore: true, nextBeforeIndex: 2),
            direction: .replaceLatest,
            savedAt: Date(timeIntervalSince1970: 0)
        )
        XCTAssertEqual(merged.messages.map(\.index), [3, 4])
        XCTAssertEqual(merged.afterCursor, 4)
        XCTAssertTrue(merged.hasMoreBefore)
        XCTAssertEqual(merged.nextBeforeIndex, 2)
    }

    func testForwardAppendsNewerAndKeepsCacheOlderBoundary() {
        let cache = GaryxCachedTranscript(
            threadId: "t",
            savedAt: Date(timeIntervalSince1970: 0),
            messages: [msg(3, .user, "u"), msg(4, .assistant, "a")],
            hasMoreBefore: true,
            nextBeforeIndex: 2
        )
        // Delta page (after_index=4) returns index 5,6; its pageInfo describes the
        // newer end and must NOT clobber the cached older boundary.
        let merged = GaryxTranscriptCacheLogic.merged(
            into: cache,
            threadId: "t",
            fetched: [msg(5, .toolUse, "tu"), msg(6, .assistant, "done")],
            pageInfo: pageInfo(hasMoreBefore: false, nextBeforeIndex: nil),
            direction: .forward,
            savedAt: Date(timeIntervalSince1970: 1)
        )
        XCTAssertEqual(merged.messages.map(\.index), [3, 4, 5, 6])
        XCTAssertEqual(merged.afterCursor, 6)
        XCTAssertTrue(merged.hasMoreBefore, "older boundary preserved across forward delta")
        XCTAssertEqual(merged.nextBeforeIndex, 2)
    }

    func testForwardDedupLetsFreshContentWinAtSameIndex() {
        // A run's terminal reconcile can rewrite the trailing row's content at the
        // same index; the fresher fetched copy must replace the cached one.
        let cache = GaryxCachedTranscript(
            threadId: "t",
            savedAt: Date(timeIntervalSince1970: 0),
            messages: [msg(0, .user, "u"), msg(1, .assistant, "partial")],
            hasMoreBefore: false,
            nextBeforeIndex: nil
        )
        let merged = GaryxTranscriptCacheLogic.merged(
            into: cache,
            threadId: "t",
            fetched: [msg(1, .assistant, "partial finalized"), msg(2, .assistant, "next")],
            pageInfo: pageInfo(),
            direction: .forward,
            savedAt: Date(timeIntervalSince1970: 1)
        )
        XCTAssertEqual(merged.messages.map(\.index), [0, 1, 2])
        XCTAssertEqual(merged.messages.first { $0.index == 1 }?.text, "partial finalized")
    }

    func testOlderPrependsAndTakesPageOlderBoundary() {
        let cache = GaryxCachedTranscript(
            threadId: "t",
            savedAt: Date(timeIntervalSince1970: 0),
            messages: [msg(5, .user, "u"), msg(6, .assistant, "a")],
            hasMoreBefore: true,
            nextBeforeIndex: 4
        )
        let merged = GaryxTranscriptCacheLogic.merged(
            into: cache,
            threadId: "t",
            fetched: [msg(3, .user, "older-u"), msg(4, .assistant, "older-a")],
            pageInfo: pageInfo(hasMoreBefore: true, nextBeforeIndex: 2),
            direction: .older,
            savedAt: Date(timeIntervalSince1970: 1)
        )
        XCTAssertEqual(merged.messages.map(\.index), [3, 4, 5, 6])
        XCTAssertEqual(merged.afterCursor, 6, "newer end unchanged when loading older")
        XCTAssertTrue(merged.hasMoreBefore)
        XCTAssertEqual(merged.nextBeforeIndex, 2, "older boundary advances to the new page")
    }

    func testMergeDropsRowsWithoutIndex() {
        let merged = GaryxTranscriptCacheLogic.merged(
            into: nil,
            threadId: "t",
            fetched: [
                msg(0, .user, "u"),
                GaryxTranscriptMessage(index: nil, role: .assistant, text: "in-flight overlay"),
                msg(1, .assistant, "a"),
            ],
            pageInfo: pageInfo(),
            direction: .replaceLatest,
            savedAt: Date(timeIntervalSince1970: 0)
        )
        XCTAssertEqual(merged.messages.map(\.index), [0, 1], "uncommitted (no index) rows are not cached")
    }

    // MARK: - file store

    func testFileStoreSaveLoadRemoveRoundTrip() throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("garyx-cache-test-\(UUID().uuidString)", isDirectory: true)
        let store = GaryxTranscriptFileCacheStore(directory: dir)
        defer { try? FileManager.default.removeItem(at: dir) }

        XCTAssertNil(store.load(threadId: "thread::x"))
        let snapshot = GaryxCachedTranscript(
            threadId: "thread::x",
            savedAt: Date(timeIntervalSince1970: 42),
            messages: [msg(0, .user, "hi"), msg(1, .assistant, "yo")],
            hasMoreBefore: false,
            nextBeforeIndex: nil
        )
        store.save(snapshot)
        XCTAssertEqual(store.load(threadId: "thread::x"), snapshot)

        // Distinct threads get distinct files (reversible base64 key, no collision).
        let other = GaryxCachedTranscript(
            threadId: "thread::y",
            savedAt: Date(timeIntervalSince1970: 7),
            messages: [msg(0, .user, "other")],
            hasMoreBefore: false,
            nextBeforeIndex: nil
        )
        store.save(other)
        XCTAssertEqual(store.load(threadId: "thread::x"), snapshot)
        XCTAssertEqual(store.load(threadId: "thread::y"), other)

        store.remove(threadId: "thread::x")
        XCTAssertNil(store.load(threadId: "thread::x"))
        XCTAssertEqual(store.load(threadId: "thread::y"), other)

        store.clearAll()
        XCTAssertNil(store.load(threadId: "thread::y"))
    }

    func testFileStoreRejectsVersionMismatch() throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("garyx-cache-test-\(UUID().uuidString)", isDirectory: true)
        let store = GaryxTranscriptFileCacheStore(directory: dir)
        defer { try? FileManager.default.removeItem(at: dir) }

        var snapshot = GaryxCachedTranscript(
            threadId: "thread::v",
            savedAt: Date(timeIntervalSince1970: 1),
            messages: [msg(0, .user, "hi")],
            hasMoreBefore: false,
            nextBeforeIndex: nil
        )
        snapshot.version = GaryxCachedTranscript.currentVersion + 1
        store.save(snapshot)
        XCTAssertNil(store.load(threadId: "thread::v"), "a future cache version is ignored, not crashed on")
    }
}
