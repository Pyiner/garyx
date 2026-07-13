import Foundation

/// Persisted committed-history window for one thread (S2 of the cursor-sync
/// design). Holds only durable committed rows — each carries a stable `index`
/// (the gateway transcript position); transient live rows are never
/// cached, so the window is always a contiguous, ascending slice of committed
/// history. The highest cached index is the forward (`after_index`) cursor used
/// for incremental open; `hasMoreBefore`/`nextBeforeIndex` extend it backward.
public struct GaryxCachedTranscript: Codable, Equatable, Sendable {
    public static let currentVersion = 1

    public var version: Int
    public var threadId: String
    public var savedAt: Date
    public var messages: [GaryxTranscriptMessage]
    public var renderSnapshot: GaryxRenderSnapshot?
    public var hasMoreBefore: Bool
    public var nextBeforeIndex: Int?

    public init(
        version: Int = Self.currentVersion,
        threadId: String,
        savedAt: Date,
        messages: [GaryxTranscriptMessage],
        renderSnapshot: GaryxRenderSnapshot? = nil,
        hasMoreBefore: Bool,
        nextBeforeIndex: Int?
    ) {
        self.version = version
        self.threadId = threadId
        self.savedAt = savedAt
        self.messages = messages
        self.renderSnapshot = renderSnapshot
        self.hasMoreBefore = hasMoreBefore
        self.nextBeforeIndex = nextBeforeIndex
    }

    /// Highest committed index in the window — the forward/delta cursor to resume
    /// from on the next incremental open. `nil` when the window is empty.
    public var afterCursor: Int? {
        messages.compactMap(\.index).max()
    }

    /// Lowest committed index — the start of the contiguous window.
    public var firstIndex: Int? {
        messages.compactMap(\.index).min()
    }

    /// True when this entry is older than `ttl` relative to `now` (`savedAt` is the
    /// last refresh time). Drives the persistent cache's validity window so a very
    /// stale window is re-fetched instead of shown on cold start.
    public func isExpired(now: Date, ttl: TimeInterval) -> Bool {
        now.timeIntervalSince(savedAt) > ttl
    }

    /// Render-input equivalence: everything `Equatable` compares except
    /// `savedAt`, which a no-op re-apply (e.g. a caught-up snapshot-only
    /// stream frame) refreshes without changing rendered output. An in-flight
    /// stream flush uses this instead of object equality so it only aborts
    /// when the prepared output could actually differ.
    public func renderEquivalent(to other: GaryxCachedTranscript) -> Bool {
        version == other.version
            && threadId == other.threadId
            && messages == other.messages
            && renderSnapshot == other.renderSnapshot
            && hasMoreBefore == other.hasMoreBefore
            && nextBeforeIndex == other.nextBeforeIndex
    }
}

/// Which end of the cached window a freshly-fetched page extends.
public enum GaryxTranscriptCacheMergeDirection: Sendable {
    /// A standalone latest page (no cursor): replaces the window with this slice.
    case replaceLatest
    /// An `after_index` delta: newer committed rows appended to the top.
    case forward
    /// A `before_index` page: older committed rows prepended to the bottom.
    case older
}

public enum GaryxTranscriptCacheLogic {
    /// Windowed-resume reset, UI side: drop on-screen rows whose durable
    /// `historyIndex` is below the window floor. Rows without a
    /// historyIndex (optimistic/pending local rows) are kept. Without this
    /// the prepared-flush preserve step (preserveRemoteBeforeIndex =
    /// window.firstIndex) re-attaches the stale prefix in front of the
    /// window (#TASK-1701 re-review).
    static func droppingLocalRowsBelow(
        floorSeq: Int,
        in messages: [GaryxMobileMessage]
    ) -> [GaryxMobileMessage] {
        let floorIndex = floorSeq - 1
        return messages.filter { message in
            guard let index = message.historyIndex else { return true }
            return index >= floorIndex
        }
    }

    /// Windowed-resume reset: drop cached committed rows below the window
    /// floor (seq is 1-based; a row's durable `index` is seq - 1). They
    /// predate the server-served window and are no longer contiguous with
    /// the connection that delivered it.
    public static func droppingCommittedBelow(
        floorSeq: Int,
        in window: GaryxCachedTranscript?
    ) -> GaryxCachedTranscript? {
        guard var window else { return nil }
        let floorIndex = floorSeq - 1
        let kept = window.messages.filter { message in
            guard let index = message.index else { return false }
            return index >= floorIndex
        }
        if kept.count == window.messages.count {
            return window
        }
        window.messages = kept
        return window
    }

    /// Keep only durable committed rows (those with a stable `index`), dedup by
    /// `index` keeping the last occurrence, ascending by `index`. A run's terminal
    /// reconcile can rewrite a row's content at the same index, so the freshest
    /// copy must win — callers pass `fetched` after `existing` for that to hold.
    static func normalizedCommitted(
        _ existing: [GaryxTranscriptMessage],
        _ fetched: [GaryxTranscriptMessage]
    ) -> [GaryxTranscriptMessage] {
        var byIndex: [Int: GaryxTranscriptMessage] = [:]
        var order: [Int] = []
        for message in existing + fetched {
            guard let index = message.index else { continue }
            if byIndex[index] == nil {
                order.append(index)
            }
            byIndex[index] = message
        }
        return order.sorted().compactMap { byIndex[$0] }
    }

    /// Merge a freshly-fetched page into the cached window, producing the new
    /// snapshot to persist. Pure — the caller decides whether to actually persist
    /// (e.g. only when the thread is idle, so no transient live row is cached).
    public static func merged(
        into cache: GaryxCachedTranscript?,
        threadId: String,
        fetched: [GaryxTranscriptMessage],
        renderSnapshot: GaryxRenderSnapshot? = nil,
        pageInfo: GaryxThreadTranscriptPageInfo?,
        direction: GaryxTranscriptCacheMergeDirection,
        savedAt: Date
    ) -> GaryxCachedTranscript {
        let base = cache?.messages ?? []
        let messages: [GaryxTranscriptMessage]
        let hasMoreBefore: Bool
        let nextBeforeIndex: Int?

        switch direction {
        case .replaceLatest:
            messages = normalizedCommitted([], fetched)
            hasMoreBefore = pageInfo?.hasMoreBefore ?? false
            nextBeforeIndex = pageInfo?.nextBeforeIndex
        case .forward:
            messages = normalizedCommitted(base, fetched)
            // The forward delta only extends the newer end; the older boundary is
            // unchanged. Fall back to the page's older info only when no cache.
            if cache != nil {
                hasMoreBefore = cache?.hasMoreBefore ?? false
                nextBeforeIndex = cache?.nextBeforeIndex
            } else {
                hasMoreBefore = pageInfo?.hasMoreBefore ?? false
                nextBeforeIndex = pageInfo?.nextBeforeIndex
            }
        case .older:
            messages = normalizedCommitted(base, fetched)
            // Loading older extends the bottom; the new older boundary comes from
            // this page.
            hasMoreBefore = pageInfo?.hasMoreBefore ?? false
            nextBeforeIndex = pageInfo?.nextBeforeIndex
        }

        return GaryxCachedTranscript(
            threadId: threadId,
            savedAt: savedAt,
            messages: messages,
            renderSnapshot: renderSnapshot ?? cache?.renderSnapshot,
            hasMoreBefore: hasMoreBefore,
            nextBeforeIndex: nextBeforeIndex
        )
    }
}

/// Persistent per-thread transcript cache. Implementations must be safe to call
/// from any actor (the app reads/writes from the main actor and background
/// fetch tasks).
public protocol GaryxTranscriptCacheStore: Sendable {
    func load(threadId: String) -> GaryxCachedTranscript?
    func save(_ snapshot: GaryxCachedTranscript)
    func remove(threadId: String)
    func clearAll()
}

/// Observable outcome of a persistent-cache write, so a failure is no longer
/// swallowed silently (TASK-1751 P5). The app wires a sink to `os.Logger`;
/// tests inject a recording closure. Persistence stays fire-and-forget for
/// callers — the gateway remains the durable source of truth — but a failed
/// write now emits a signal and never leaks its temporary file.
public enum GaryxTranscriptCacheStoreEvent: Equatable, Sendable {
    /// The snapshot could not be JSON-encoded; nothing was written.
    case saveEncodeFailed(threadId: String)
    /// The encoded snapshot could not be written/atomically replaced on disk.
    case saveWriteFailed(threadId: String, reason: String)
}

/// File-backed cache: one JSON file per thread under `directory`, named by a
/// reversible URL-safe base64 of the thread id (thread ids contain `::`). One
/// file per thread keeps writes O(one thread) and avoids loading every thread's
/// transcript to update one — important when a single thread can be tens of MB.
public final class GaryxTranscriptFileCacheStore: GaryxTranscriptCacheStore, @unchecked Sendable {
    /// Default validity window for persisted entries: one day. A window older than
    /// this is treated as absent (re-fetched) rather than shown stale on cold start.
    public static let defaultTTL: TimeInterval = 24 * 60 * 60

    private let directory: URL
    private let fileManager: FileManager
    private let ttl: TimeInterval?
    private let now: () -> Date
    private let diagnostics: (@Sendable (GaryxTranscriptCacheStoreEvent) -> Void)?
    private let lock = NSLock()
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder

    public init(
        directory: URL,
        ttl: TimeInterval? = nil,
        now: @escaping () -> Date = { Date() },
        fileManager: FileManager = .default,
        diagnostics: (@Sendable (GaryxTranscriptCacheStoreEvent) -> Void)? = nil
    ) {
        self.directory = directory
        self.ttl = ttl
        self.now = now
        self.fileManager = fileManager
        self.diagnostics = diagnostics
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        self.encoder = encoder
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        self.decoder = decoder
        try? fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        // Sweep orphan temporary files left by an older app version or by a crash
        // between `data.write(to: tmp)` and the atomic replace (TASK-1751 P5) —
        // nothing else ever removes them, so they would accumulate forever. Safe
        // at init: this instance owns the single-writer lock and no save can run
        // before construction returns.
        sweepOrphanTemporaryFiles()
        // Sweep entries already past their validity window on startup so the cache
        // never grows unbounded with stale threads.
        pruneExpired()
    }

    /// Default location under the app caches directory. Cache (not Documents) so
    /// the OS may evict it under storage pressure — it is always re-derivable.
    public static func defaultDirectory(fileManager: FileManager = .default) -> URL {
        let base = fileManager.urls(for: .cachesDirectory, in: .userDomainMask).first
            ?? fileManager.temporaryDirectory
        return base.appendingPathComponent("garyx-transcripts", isDirectory: true)
    }

    private func fileURL(threadId: String) -> URL {
        directory.appendingPathComponent("\(cacheKey(threadId: threadId)).json", isDirectory: false)
    }

    private func cacheKey(threadId: String) -> String {
        Data(threadId.utf8)
            .base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    public func load(threadId: String) -> GaryxCachedTranscript? {
        lock.lock()
        defer { lock.unlock() }
        let url = fileURL(threadId: threadId)
        guard let data = try? Data(contentsOf: url),
              let snapshot = try? decoder.decode(GaryxCachedTranscript.self, from: data),
              snapshot.version == GaryxCachedTranscript.currentVersion,
              snapshot.threadId == threadId
        else {
            return nil
        }
        if let ttl, snapshot.isExpired(now: now(), ttl: ttl) {
            try? fileManager.removeItem(at: url)
            return nil
        }
        return snapshot
    }

    public func save(_ snapshot: GaryxCachedTranscript) {
        lock.lock()
        defer { lock.unlock() }
        let threadId = snapshot.threadId
        guard let data = try? encoder.encode(snapshot) else {
            // Encoding failure is a programmer/data error, not a disk fault:
            // surface it instead of silently dropping the write.
            diagnostics?(.saveEncodeFailed(threadId: threadId))
            return
        }
        let url = fileURL(threadId: threadId)
        let tmp = url.appendingPathExtension("tmp")
        do {
            try data.write(to: tmp, options: .atomic)
            // Replace the live file atomically so a crash mid-write cannot leave a
            // truncated cache that would mask real history on next open. The
            // replace joins the same do/catch as the write: a failing replace
            // must clean up its temporary file and report, not swallow the error
            // and leak a `.json.tmp` that nothing ever sweeps (TASK-1751 P5).
            if fileManager.fileExists(atPath: url.path) {
                _ = try fileManager.replaceItemAt(url, withItemAt: tmp)
            } else {
                try fileManager.moveItem(at: tmp, to: url)
            }
        } catch {
            try? fileManager.removeItem(at: tmp)
            diagnostics?(.saveWriteFailed(
                threadId: threadId,
                reason: (error as NSError).localizedDescription
            ))
        }
    }

    public func remove(threadId: String) {
        lock.lock()
        defer { lock.unlock() }
        let url = fileURL(threadId: threadId)
        try? fileManager.removeItem(at: url)
        // Drop the matching temporary sibling too, so a failed/interrupted save
        // never leaves a `<key>.json.tmp` behind after the thread is removed.
        try? fileManager.removeItem(at: url.appendingPathExtension("tmp"))
    }

    public func clearAll() {
        lock.lock()
        defer { lock.unlock() }
        guard let entries = try? fileManager.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: nil
        ) else { return }
        // Remove both committed caches and any temporary residue.
        for entry in entries where entry.pathExtension == "json" || entry.pathExtension == "tmp" {
            try? fileManager.removeItem(at: entry)
        }
    }

    /// Remove entries past their validity window (`ttl`) and any orphan temporary
    /// files. The tmp sweep runs regardless of TTL. Called on init; also reusable
    /// for an explicit sweep.
    public func pruneExpired() {
        lock.lock()
        defer { lock.unlock() }
        sweepOrphanTemporaryFilesLocked()
        guard let ttl else { return }
        let nowValue = now()
        guard let entries = try? fileManager.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: nil
        ) else { return }
        for entry in entries where entry.pathExtension == "json" {
            guard let data = try? Data(contentsOf: entry),
                  let snapshot = try? decoder.decode(GaryxCachedTranscript.self, from: data)
            else {
                continue
            }
            if snapshot.isExpired(now: nowValue, ttl: ttl) {
                try? fileManager.removeItem(at: entry)
            }
        }
    }

    /// Remove every orphan `*.json.tmp` (leaked by an older version, a crash
    /// between the tmp write and the atomic replace, or an interrupted save).
    /// Acquires the store lock.
    private func sweepOrphanTemporaryFiles() {
        lock.lock()
        defer { lock.unlock() }
        sweepOrphanTemporaryFilesLocked()
    }

    /// Lock-held tmp sweep so callers already holding `lock` reuse it.
    private func sweepOrphanTemporaryFilesLocked() {
        guard let entries = try? fileManager.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: nil
        ) else { return }
        for entry in entries where entry.pathExtension == "tmp" {
            try? fileManager.removeItem(at: entry)
        }
    }
}
