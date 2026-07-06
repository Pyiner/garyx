import XCTest
@testable import GaryxMobileCore

/// TASK-1751 reproductions: deterministic, no-UI quantification of the audited
/// mobile chat pipeline problems. These tests build a large committed
/// transcript comparable to a real long-running thread (tens of MB of cache
/// JSON) and measure the exact operations the audited call sites perform.
///
/// P1 — `showSelectedThread` (main actor) synchronously runs
///      `restoredCachedMessages` → `transcriptSnapshot` → `store.load()`
///      (`Data(contentsOf:)` + `JSONDecoder.decode`) + full
///      `mobileMessages(from:)` mapping. `testP1_coldOpenSyncRestoreCost…`
///      measures that exact work.
/// P2 — the conversation body and its `.onChange` each call
///      `selectedThreadTurnRows()` per SwiftUI body evaluation; the mapper
///      rebuilds `MessageLookup` (re-mapping every transcript message) every
///      call. `testP2_turnRowsMapperRebuildCost…` measures one such call and
///      verifies the mapper is pure (same input → same output), which is the
///      precondition for signature-keyed caching.
final class GaryxChatPipelineReproTests: XCTestCase {
    // MARK: - Fixture

    /// ~large committed thread: `turns` user↔assistant exchanges with tool
    /// activity, several KB of text per row — the shape long agent threads
    /// take. 700 turns ≈ 2.8k rows ≈ tens of MB of cache JSON.
    static func makeLargeWindow(
        threadId: String = "thread::repro",
        turns: Int = 700,
        bodyKB: Int = 4
    ) -> GaryxCachedTranscript {
        let filler = String(repeating: "The quick brown fox jumps over the lazy dog. ", count: max(1, bodyKB * 1024 / 46))
        var messages: [GaryxTranscriptMessage] = []
        var rows: [GaryxRenderRow] = []
        messages.reserveCapacity(turns * 4)
        rows.reserveCapacity(turns)
        var index = 0
        for turn in 0..<turns {
            let userIndex = index
            messages.append(GaryxTranscriptMessage(
                index: userIndex,
                role: .user,
                text: "question \(turn): \(filler.prefix(512))"
            ))
            index += 1
            let toolCallIndex = index
            messages.append(GaryxTranscriptMessage(
                index: toolCallIndex,
                role: .assistant,
                text: "",
                toolRelated: true,
                toolName: "Bash",
                toolUseId: "tool_\(turn)"
            ))
            index += 1
            let toolResultIndex = index
            messages.append(GaryxTranscriptMessage(
                index: toolResultIndex,
                role: .tool,
                text: "tool output \(turn): \(filler.prefix(1024))",
                toolRelated: true,
                toolName: "Bash",
                toolUseResult: true,
                toolUseId: "tool_\(turn)"
            ))
            index += 1
            let assistantIndex = index
            messages.append(GaryxTranscriptMessage(
                index: assistantIndex,
                role: .assistant,
                text: "answer \(turn): \(filler)"
            ))
            index += 1
            rows.append(.userTurn(GaryxRenderUserTurnRow(
                id: "turn:\(userIndex + 1)",
                user: GaryxRenderMessageRef(id: "seq:\(userIndex + 1)", seq: userIndex + 1, role: "user"),
                activity: [
                    .step(GaryxRenderStepRow(
                        id: "step:\(toolCallIndex + 1)",
                        steps: [
                            .toolGroup(GaryxRenderToolGroup(
                                id: "group:\(toolCallIndex + 1)",
                                status: .completed,
                                entries: [
                                    GaryxRenderToolEntry(
                                        id: "entry:\(toolCallIndex + 1)",
                                        toolUseId: "tool_\(turn)",
                                        status: .completed,
                                        toolUse: GaryxRenderMessageRef(
                                            id: "seq:\(toolCallIndex + 1)",
                                            seq: toolCallIndex + 1,
                                            role: "assistant"
                                        ),
                                        toolResult: GaryxRenderMessageRef(
                                            id: "seq:\(toolResultIndex + 1)",
                                            seq: toolResultIndex + 1,
                                            role: "tool"
                                        )
                                    ),
                                ]
                            )),
                        ],
                        finalMessage: GaryxRenderMessageRef(
                            id: "seq:\(assistantIndex + 1)",
                            seq: assistantIndex + 1,
                            role: "assistant"
                        ),
                        running: false
                    )),
                ]
            )))
        }
        return GaryxCachedTranscript(
            threadId: threadId,
            savedAt: Date(),
            messages: messages,
            renderSnapshot: GaryxRenderSnapshot(basedOnSeq: index, rows: rows),
            hasMoreBefore: false,
            nextBeforeIndex: nil
        )
    }

    private func makeStoreDirectory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("garyx-repro-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock {
            try? FileManager.default.removeItem(at: directory)
        }
        return directory
    }

    private func measureMs(_ body: () -> Void) -> Double {
        let start = DispatchTime.now().uptimeNanoseconds
        body()
        return Double(DispatchTime.now().uptimeNanoseconds - start) / 1_000_000
    }

    // MARK: - P1: cold-open synchronous disk restore cost

    /// Quantifies the exact synchronous work `showSelectedThread` performs on
    /// the main actor for a cold open with a large persisted cache:
    /// `store.load` (read + JSON decode) plus the full `mobileMessages`
    /// mapping. A 60fps main-thread frame budget is 16ms; this must not run
    /// synchronously on the main actor if it exceeds one frame.
    func testP1_coldOpenSyncRestoreCostExceedsFrameBudget() throws {
        let directory = try makeStoreDirectory()
        let store = GaryxTranscriptFileCacheStore(directory: directory)
        let window = Self.makeLargeWindow()
        store.save(window)

        let fileURL = try XCTUnwrap(
            FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.fileSizeKey])
                .first(where: { $0.pathExtension == "json" })
        )
        let fileSizeMB = Double((try fileURL.resourceValues(forKeys: [.fileSizeKey])).fileSize ?? 0) / 1_048_576

        var loaded: GaryxCachedTranscript?
        let loadMs = measureMs {
            loaded = store.load(threadId: window.threadId)
        }
        let snapshot = try XCTUnwrap(loaded)

        var mapped: [GaryxMobileMessage] = []
        let mapMs = measureMs {
            mapped = GaryxMobileTranscriptMapper.mobileMessages(from: snapshot.messages, live: false)
        }
        XCTAssertFalse(mapped.isEmpty)

        print("[TASK-1751 P1] cache file: \(String(format: "%.1f", fileSizeMB)) MB, " +
              "rows: \(snapshot.messages.count), store.load: \(String(format: "%.1f", loadMs)) ms, " +
              "mobileMessages map: \(String(format: "%.1f", mapMs)) ms, " +
              "total sync main-actor cost: \(String(format: "%.1f", loadMs + mapMs)) ms")

        XCTAssertGreaterThan(
            loadMs + mapMs,
            16,
            "cold-open restore cost is expected to exceed one 60fps frame; if this ever " +
            "gets cheaper than a frame the synchronous-restore ban can be revisited"
        )
    }

    // MARK: - P2: per-body-evaluation full mapper rebuild cost

    /// Quantifies one `selectedThreadTurnRows()`-equivalent call: the mapper
    /// rebuilds the whole `MessageLookup` (including re-mapping every
    /// transcript message through `GaryxMobileTranscriptMapper`) and re-derives
    /// every row. The conversation view performs this at least twice per body
    /// evaluation (body + `.onChange(of:)`), on every SwiftUI invalidation.
    func testP2_turnRowsMapperRebuildCostAndPurity() throws {
        let window = Self.makeLargeWindow()
        let mobile = GaryxMobileTranscriptMapper.mobileMessages(from: window.messages, live: false)

        var rows: [GaryxMobileTurnRow] = []
        let firstMs = measureMs {
            rows = GaryxMobileRenderStateMapper.rows(
                snapshot: window.renderSnapshot,
                messages: mobile,
                transcriptMessages: window.messages
            )
        }
        var rowsAgain: [GaryxMobileTurnRow] = []
        let secondMs = measureMs {
            rowsAgain = GaryxMobileRenderStateMapper.rows(
                snapshot: window.renderSnapshot,
                messages: mobile,
                transcriptMessages: window.messages
            )
        }

        print("[TASK-1751 P2] turn rows: \(rows.count), mapper full rebuild: " +
              "\(String(format: "%.1f", firstMs)) ms first / \(String(format: "%.1f", secondMs)) ms repeat " +
              "(body + onChange ⇒ ≥2 rebuilds per SwiftUI body evaluation)")

        // Purity: identical inputs produce identical rows — the precondition
        // for caching prepared rows keyed by input identity.
        XCTAssertEqual(rows, rowsAgain, "mapper must be a pure function of its inputs to be cacheable")
        XCTAssertGreaterThan(
            firstMs + secondMs,
            16,
            "two mapper rebuilds (one body evaluation) are expected to exceed a frame budget on large threads"
        )
    }
}

/// TASK-1751 P5 — persistent cache writes fail silently: `save()` wraps
/// `replaceItemAt` in `_ = try?`, so a replace failure neither surfaces any
/// signal nor cleans up the temporary file, and the stale previous window
/// stays on disk masquerading as current.
final class GaryxTranscriptCachePersistFailureReproTests: XCTestCase {
    /// Injects a deterministic failure into the atomic-replace step — the
    /// exact failure mode `_ = try?` swallows in production (e.g. volume
    /// full, sandbox/permission churn, iCloud eviction races).
    final class ReplaceFailingFileManager: FileManager {
        var replaceAttempts = 0

        override func replaceItem(
            at originalItemURL: URL,
            withItemAt newItemURL: URL,
            backupItemName: String?,
            options: FileManager.ItemReplacementOptions = [],
            resultingItemURL: AutoreleasingUnsafeMutablePointer<NSURL?>?
        ) throws {
            replaceAttempts += 1
            throw CocoaError(.fileWriteVolumeReadOnly)
        }
    }

    private func makeStoreDirectory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("garyx-repro-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock {
            try? FileManager.default.removeItem(at: directory)
        }
        return directory
    }

    private func cacheFileURL(directory: URL, threadId: String) -> URL {
        let key = Data(threadId.utf8)
            .base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
        return directory.appendingPathComponent("\(key).json", isDirectory: false)
    }

    private func makeWindow(threadId: String, text: String) -> GaryxCachedTranscript {
        GaryxCachedTranscript(
            threadId: threadId,
            savedAt: Date(),
            messages: [GaryxTranscriptMessage(index: 0, role: .assistant, text: text)],
            hasMoreBefore: false,
            nextBeforeIndex: nil
        )
    }

    /// A failed replace of the live cache file must not leak the temporary
    /// file next to the cache (leaked `.json.tmp` files accumulate forever —
    /// nothing ever sweeps them).
    func testP5_saveReplaceFailureCleansUpTemporaryFile() throws {
        let directory = try makeStoreDirectory()
        let failingFileManager = ReplaceFailingFileManager()
        let store = GaryxTranscriptFileCacheStore(directory: directory, fileManager: failingFileManager)
        let threadId = "thread::persist-failure"
        let url = cacheFileURL(directory: directory, threadId: threadId)

        // Seed a valid previous window so the live file exists and save()
        // takes the replaceItemAt branch.
        store.save(makeWindow(threadId: threadId, text: "old content"))
        XCTAssertTrue(FileManager.default.fileExists(atPath: url.path))

        store.save(makeWindow(threadId: threadId, text: "new content"))

        XCTAssertGreaterThan(failingFileManager.replaceAttempts, 0, "injection must reach the replace step")
        let tmp = url.appendingPathExtension("tmp")
        XCTAssertFalse(
            FileManager.default.fileExists(atPath: tmp.path),
            "a failed replace must clean up its temporary file instead of leaking it next to the cache"
        )
    }

    /// A write failure must surface a diagnostics event carrying the thread id
    /// and a reason — the fix's observability contract.
    func testP5_saveFailureEmitsWriteDiagnostics() throws {
        let directory = try makeStoreDirectory()
        let failingFileManager = ReplaceFailingFileManager()
        var events: [GaryxTranscriptCacheStoreEvent] = []
        let lock = NSLock()
        let store = GaryxTranscriptFileCacheStore(
            directory: directory,
            fileManager: failingFileManager,
            diagnostics: { event in
                lock.lock(); defer { lock.unlock() }
                events.append(event)
            }
        )
        let threadId = "thread::persist-diagnostics"

        store.save(makeWindow(threadId: threadId, text: "old content"))
        // First save creates the file (moveItem branch, not replace) → no event.
        XCTAssertTrue(events.isEmpty, "a successful write must not emit a failure event")

        store.save(makeWindow(threadId: threadId, text: "new content"))

        guard case let .saveWriteFailed(reportedThreadId, reason)? = events.first else {
            return XCTFail("expected a saveWriteFailed event, got \(events)")
        }
        XCTAssertEqual(reportedThreadId, threadId)
        XCTAssertFalse(reason.isEmpty, "the failure reason must be carried for logging")
    }

    /// A successful save must never emit a failure event (guards against a
    /// diagnostics sink that fires on the happy path).
    func testP5_successfulSaveEmitsNoDiagnostics() throws {
        let directory = try makeStoreDirectory()
        var events: [GaryxTranscriptCacheStoreEvent] = []
        let store = GaryxTranscriptFileCacheStore(
            directory: directory,
            diagnostics: { events.append($0) }
        )
        let threadId = "thread::persist-ok"
        store.save(makeWindow(threadId: threadId, text: "first"))
        store.save(makeWindow(threadId: threadId, text: "second (replace path)"))
        XCTAssertEqual(store.load(threadId: threadId)?.messages.first?.text, "second (replace path)")
        XCTAssertTrue(events.isEmpty, "successful saves (create + atomic replace) must be silent")
    }

    /// Orphan `.json.tmp` residue (older version / crash between tmp write and
    /// replace) is swept when a store is constructed over the directory.
    func testP5_initSweepsOrphanTmpFiles() throws {
        let directory = try makeStoreDirectory()
        let orphanTmp = directory.appendingPathComponent("SGVsbG8.json.tmp")
        let unrelated = directory.appendingPathComponent("keep.json")
        try Data("stale tmp".utf8).write(to: orphanTmp)
        try Data("{}".utf8).write(to: unrelated)

        _ = GaryxTranscriptFileCacheStore(directory: directory)

        XCTAssertFalse(FileManager.default.fileExists(atPath: orphanTmp.path), "init must sweep orphan .json.tmp")
        XCTAssertTrue(FileManager.default.fileExists(atPath: unrelated.path), "init must not touch .json files")
    }

    /// `remove(threadId:)` drops the thread's tmp sibling too.
    func testP5_removeSweepsThreadTmp() throws {
        let directory = try makeStoreDirectory()
        let store = GaryxTranscriptFileCacheStore(directory: directory)
        let threadId = "thread::remove-tmp"
        store.save(makeWindow(threadId: threadId, text: "content"))
        let url = cacheFileURL(directory: directory, threadId: threadId)
        let tmp = url.appendingPathExtension("tmp")
        try Data("leaked".utf8).write(to: tmp)

        store.remove(threadId: threadId)

        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: tmp.path), "remove must drop the tmp sibling")
    }

    /// `clearAll()` removes tmp residue alongside committed caches.
    func testP5_clearAllSweepsTmp() throws {
        let directory = try makeStoreDirectory()
        let store = GaryxTranscriptFileCacheStore(directory: directory)
        store.save(makeWindow(threadId: "thread::a", text: "a"))
        store.save(makeWindow(threadId: "thread::b", text: "b"))
        let orphanTmp = directory.appendingPathComponent("Zm9v.json.tmp")
        try Data("leaked".utf8).write(to: orphanTmp)

        store.clearAll()

        let remaining = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == "json" || $0.pathExtension == "tmp" }
        XCTAssertTrue(remaining.isEmpty, "clearAll must remove both .json and .json.tmp, got \(remaining)")
    }
}
