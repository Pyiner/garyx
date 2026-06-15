import XCTest
@testable import GaryxMobileCore

/// UI-free coverage of the persistent transcript cache's validity window: the pure
/// `isExpired` check and the file store honoring a TTL on load + pruning on init,
/// all with an injected clock (no real time, no simulator).
final class GaryxTranscriptCacheTTLTests: XCTestCase {
    private let day: TimeInterval = 24 * 60 * 60

    private func window(_ threadId: String, savedAt: Date) -> GaryxCachedTranscript {
        GaryxCachedTranscript(
            threadId: threadId,
            savedAt: savedAt,
            messages: [],
            hasMoreBefore: false,
            nextBeforeIndex: nil
        )
    }

    private func tempDir() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("garyx-ttl-\(UUID().uuidString)", isDirectory: true)
    }

    // MARK: - isExpired (pure)

    func testIsExpiredFreshIsFalse() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        XCTAssertFalse(window("t", savedAt: now.addingTimeInterval(-3_600)).isExpired(now: now, ttl: day))
    }

    func testIsExpiredOldIsTrue() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        XCTAssertTrue(window("t", savedAt: now.addingTimeInterval(-90_000)).isExpired(now: now, ttl: day))
    }

    func testIsExpiredAtBoundaryIsNotExpired() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        // Exactly `ttl` old is not expired (strictly-greater comparison).
        XCTAssertFalse(window("t", savedAt: now.addingTimeInterval(-day)).isExpired(now: now, ttl: day))
    }

    // MARK: - File store load honors the TTL

    func testLoadReturnsFreshEntry() {
        let t0 = Date(timeIntervalSince1970: 2_000_000)
        var clock = t0
        let store = GaryxTranscriptFileCacheStore(directory: tempDir(), ttl: day, now: { clock })
        store.save(window("thread::a", savedAt: t0))
        clock = t0.addingTimeInterval(3_600) // 1h later
        XCTAssertNotNil(store.load(threadId: "thread::a"))
    }

    func testLoadDropsAndDeletesExpiredEntry() {
        let t0 = Date(timeIntervalSince1970: 2_000_000)
        var clock = t0
        let store = GaryxTranscriptFileCacheStore(directory: tempDir(), ttl: day, now: { clock })
        store.save(window("thread::a", savedAt: t0))
        clock = t0.addingTimeInterval(90_000) // 25h later → expired
        XCTAssertNil(store.load(threadId: "thread::a"))
        // The expired file was removed on load, so even rewinding the clock it stays gone.
        clock = t0
        XCTAssertNil(store.load(threadId: "thread::a"))
    }

    func testNoTTLKeepsEntryIndefinitely() {
        let t0 = Date(timeIntervalSince1970: 2_000_000)
        var clock = t0
        let store = GaryxTranscriptFileCacheStore(directory: tempDir(), ttl: nil, now: { clock })
        store.save(window("thread::a", savedAt: t0))
        clock = t0.addingTimeInterval(10 * day)
        XCTAssertNotNil(store.load(threadId: "thread::a"))
    }

    // MARK: - pruneExpired on init

    func testInitPrunesExpiredKeepsFresh() {
        let dir = tempDir()
        let t0 = Date(timeIntervalSince1970: 3_000_000)
        // Seed two entries via a no-TTL store so nothing is pruned at seed time.
        let seeder = GaryxTranscriptFileCacheStore(directory: dir, ttl: nil, now: { t0 })
        seeder.save(window("thread::fresh", savedAt: t0))
        seeder.save(window("thread::old", savedAt: t0.addingTimeInterval(-90_000))) // 25h old
        // A TTL store created at t0 prunes the stale entry on init.
        let store = GaryxTranscriptFileCacheStore(directory: dir, ttl: day, now: { t0 })
        XCTAssertNotNil(store.load(threadId: "thread::fresh"))
        XCTAssertNil(store.load(threadId: "thread::old"))
    }
}
