import Foundation
import UIKit

/// Small in-memory decoded-image cache for capsule thumbnails, keyed by the
/// `(id, revision, rendition)` storage token. Avoids re-decoding the PNG from
/// disk on every cell appearance while scrolling. Capacity-bounded (insertion
/// order) and supports evicting every entry of one capsule on a `/serve` 404.
@MainActor
final class GaryxCapsuleThumbnailMemoryCache {
    private var images: [String: UIImage] = [:]
    private var order: [String] = []
    private let capacity: Int

    init(capacity: Int = 80) { self.capacity = max(1, capacity) }

    func image(for key: GaryxCapsuleThumbnailCacheKey) -> UIImage? {
        images[key.storageToken]
    }

    func set(_ image: UIImage, for key: GaryxCapsuleThumbnailCacheKey) {
        let token = key.storageToken
        if images[token] == nil { order.append(token) }
        images[token] = image
        while order.count > capacity {
            let oldest = order.removeFirst()
            images.removeValue(forKey: oldest)
        }
    }

    /// Evict every cached rendition/revision of one capsule (a `/serve` 404).
    /// Returns whether anything was evicted so the caller can bump the epoch.
    @discardableResult
    func evict(capsuleId: String) -> Bool {
        let id = capsuleId.trimmingCharacters(in: .whitespacesAndNewlines)
        let prefix = id + "."
        let dropped = order.filter { $0.hasPrefix(prefix) }
        guard !dropped.isEmpty else { return false }
        for token in dropped { images.removeValue(forKey: token) }
        order.removeAll { $0.hasPrefix(prefix) }
        return true
    }

    /// Drop entries for capsules no longer in the authoritative list (a remote
    /// delete surfaced via the `capsules` refresh). Returns whether anything was
    /// evicted so the caller can bump the cache epoch.
    @discardableResult
    func retainOnly(validIds: Set<String>) -> Bool {
        let dropped = order.filter { token in
            guard let dot = token.firstIndex(of: ".") else { return true }
            return !validIds.contains(String(token[token.startIndex..<dot]))
        }
        guard !dropped.isEmpty else { return false }
        let droppedSet = Set(dropped)
        for token in dropped { images.removeValue(forKey: token) }
        order.removeAll { droppedSet.contains($0) }
        return true
    }
}

/// On-disk cache of *rendered* capsule thumbnail PNGs, keyed by
/// `(id, revision, rendition)`. The gallery and chat cards render the cached
/// image (zero live `WKWebView`), so steady-state browsing mounts no web views
/// — the live render happens once, on first sight or after a `revision` bump.
///
/// Mirrors `GaryxAvatarDiskStore`: an actor over an App Group container with a
/// small JSON index, atomic writes, backup exclusion, and an LRU byte/record
/// cap. Eviction decisions reuse the pure `GaryxCapsuleThumbnailCachePruner`.
actor GaryxCapsuleThumbnailDiskStore {
    struct Entry: Codable, Sendable {
        var id: String
        var revision: Int
        var aspectWidth: Int
        var aspectHeight: Int
        var fileName: String
        var byteCount: Int
        var lastAccessAt: Date

        var cacheKey: GaryxCapsuleThumbnailCacheKey {
            GaryxCapsuleThumbnailCacheKey(
                id: id,
                revision: revision,
                rendition: GaryxCapsuleThumbnailRendition(aspectWidth: aspectWidth, aspectHeight: aspectHeight)
            )
        }
    }

    private let directory: URL
    private let maxBytes: Int
    private let maxRecords: Int
    private var index: [String: Entry] = [:] // storageToken -> Entry
    private var warmed = false

    init(directory: URL? = nil, maxBytes: Int = 48 * 1024 * 1024, maxRecords: Int = 240) {
        self.directory = directory ?? GaryxCapsuleThumbnailDiskStore.defaultDirectory()
        self.maxBytes = maxBytes
        self.maxRecords = maxRecords
    }

    static func defaultDirectory() -> URL {
        let fileManager = FileManager.default
        let base = fileManager.containerURL(
            forSecurityApplicationGroupIdentifier: GaryxMobileWidgetStore.appGroupIdentifier
        ) ?? fileManager.urls(for: .cachesDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
        return base
            .appendingPathComponent("GaryxCapsuleThumbnailCache", isDirectory: true)
            .appendingPathComponent("v1", isDirectory: true)
    }

    private var indexURL: URL { directory.appendingPathComponent("index.json", isDirectory: false) }

    func warm() {
        guard !warmed else { return }
        warmed = true
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try excludeFromBackup(directory)
            let data = try Data(contentsOf: indexURL)
            index = (try? JSONDecoder().decode([String: Entry].self, from: data)) ?? [:]
        } catch {
            index = [:]
        }
        evictStaleSchemaEntries()
    }

    /// Drop renders from a previous schema version (the renderer changed and
    /// `GaryxCapsuleThumbnailRenderSchema.version` was bumped), so the stale
    /// images re-render instead of being served. Each entry's key is rebuilt
    /// from stored metadata and carries the current schema; a token written
    /// under an older schema (or a legacy token with no schema suffix) differs.
    private func evictStaleSchemaEntries() {
        let entries = index.map { (token: $0.key, key: $0.value.cacheKey) }
        let split = GaryxCapsuleThumbnailCachePruner.evictingStaleSchema(entries: entries)
        guard !split.evictTokens.isEmpty else { return }
        for token in split.evictTokens {
            if let entry = index.removeValue(forKey: token) {
                let url = directory.appendingPathComponent(entry.fileName, isDirectory: false)
                try? FileManager.default.removeItem(at: url)
            }
        }
        writeIndex()
    }

    /// Cached PNG bytes for a key, touching last-access for LRU. Returns nil on a
    /// miss or if the backing file vanished (self-healing: the stale index entry
    /// is dropped).
    func data(for key: GaryxCapsuleThumbnailCacheKey, now: Date = Date()) -> Data? {
        warm()
        guard var entry = index[key.storageToken] else { return nil }
        let url = directory.appendingPathComponent(entry.fileName, isDirectory: false)
        guard let data = try? Data(contentsOf: url) else {
            index.removeValue(forKey: key.storageToken)
            writeIndex()
            return nil
        }
        entry.lastAccessAt = now
        index[key.storageToken] = entry
        return data
    }

    func store(_ data: Data, for key: GaryxCapsuleThumbnailCacheKey, now: Date = Date()) {
        warm()
        let fileName = key.storageToken + ".png"
        let url = directory.appendingPathComponent(fileName, isDirectory: false)
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try data.write(to: url, options: [.atomic])
            index[key.storageToken] = Entry(
                id: key.id,
                revision: key.revision,
                aspectWidth: key.rendition.aspectWidth,
                aspectHeight: key.rendition.aspectHeight,
                fileName: fileName,
                byteCount: data.count,
                lastAccessAt: now
            )
            pruneToLimits()
            writeIndex()
        } catch {
            // Best-effort cache: a failed write just means the next sighting re-renders.
        }
    }

    /// Evict every cached rendition/revision of one capsule (a `/serve` 404).
    @discardableResult
    func evict(capsuleId: String) -> Bool {
        warm()
        let split = GaryxCapsuleThumbnailCachePruner.evictingCapsule(keys: indexedKeys(), capsuleId: capsuleId)
        return removeKeys(split.evict)
    }

    /// Drop superseded revisions and deleted capsules against the authoritative
    /// list (called from the `capsules` didSet alongside the HTML prune).
    @discardableResult
    func pruneToValid(_ validCapsules: [GaryxCapsuleSummary]) -> Bool {
        warm()
        let split = GaryxCapsuleThumbnailCachePruner.pruned(keys: indexedKeys(), validCapsules: validCapsules)
        return removeKeys(split.evict)
    }

    // MARK: - Internals

    private func indexedKeys() -> [GaryxCapsuleThumbnailCacheKey] {
        index.values.map { $0.cacheKey }
    }

    @discardableResult
    private func removeKeys(_ keys: [GaryxCapsuleThumbnailCacheKey]) -> Bool {
        guard !keys.isEmpty else { return false }
        for key in keys {
            if let entry = index.removeValue(forKey: key.storageToken) {
                let url = directory.appendingPathComponent(entry.fileName, isDirectory: false)
                try? FileManager.default.removeItem(at: url)
            }
        }
        writeIndex()
        return true
    }

    /// LRU eviction to the byte/record cap (oldest last-access first).
    private func pruneToLimits() {
        var totalBytes = index.values.reduce(0) { $0 + $1.byteCount }
        guard index.count > maxRecords || totalBytes > maxBytes else { return }
        let ordered = index.values.sorted { $0.lastAccessAt < $1.lastAccessAt }
        var cursor = 0
        while (index.count > maxRecords || totalBytes > maxBytes) && cursor < ordered.count {
            let entry = ordered[cursor]
            cursor += 1
            guard index.removeValue(forKey: entry.cacheKey.storageToken) != nil else { continue }
            totalBytes -= entry.byteCount
            let url = directory.appendingPathComponent(entry.fileName, isDirectory: false)
            try? FileManager.default.removeItem(at: url)
        }
    }

    private func writeIndex() {
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let data = try JSONEncoder().encode(index)
            try data.write(to: indexURL, options: [.atomic])
        } catch {
            // Index is a reconstructable convenience; a failed flush is non-fatal.
        }
    }

    private func excludeFromBackup(_ url: URL) throws {
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        var mutableURL = url
        try mutableURL.setResourceValues(values)
    }
}
