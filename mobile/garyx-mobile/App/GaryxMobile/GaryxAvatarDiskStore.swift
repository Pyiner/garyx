import Foundation
import ImageIO
import SwiftUI
import UIKit

struct GaryxAvatarCGImageValidator: GaryxAvatarImageValidating {
    func validate(payload: GaryxAvatarPayload) -> Bool {
        let options = [kCGImageSourceShouldCache: false] as CFDictionary
        guard let source = CGImageSourceCreateWithData(payload.data as CFData, options),
              CGImageSourceGetCount(source) > 0,
              CGImageSourceCreateImageAtIndex(source, 0, options) != nil else {
            return false
        }
        return true
    }
}

actor GaryxAvatarDiskStore: GaryxAvatarStore {
    private let directory: URL
    private let policy: GaryxAvatarPruningPolicy
    private var index = GaryxAvatarStoreIndex()
    private var warmed = false
    private var indexDirty = false
    private var deferredIndexFlushTask: Task<Void, Never>?

    init(directory: URL? = nil, policy: GaryxAvatarPruningPolicy = .default) {
        self.directory = directory ?? GaryxAvatarDiskStore.defaultDirectory()
        self.policy = policy
    }

    static func defaultDirectory() -> URL {
        let fileManager = FileManager.default
        let base = fileManager.containerURL(
            forSecurityApplicationGroupIdentifier: GaryxMobileWidgetStore.appGroupIdentifier
        ) ?? fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
        return base
            .appendingPathComponent("GaryxAvatarCache", isDirectory: true)
            .appendingPathComponent("v1", isDirectory: true)
    }

    func warm() async {
        guard !warmed else { return }
        warmed = true
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try excludeFromBackup(directory)
            let data = try Data(contentsOf: indexURL)
            index = GaryxAvatarStoreIndex.decodeCurrent(from: data) ?? GaryxAvatarStoreIndex()
        } catch {
            index = GaryxAvatarStoreIndex()
        }
    }

    func storedAvatar(for identity: GaryxAvatarIdentity, now: Date = Date()) async -> GaryxStoredAvatar? {
        await warm()
        let key = identity.storageKey
        guard var entry = index.entries[key] else { return nil }
        let url = directory.appendingPathComponent(entry.fileName, isDirectory: false)
        do {
            let data = try Data(contentsOf: url)
            entry.lastAccessAt = now
            entry.byteCount = data.count
            index.entries[key] = entry
            markIndexDirtyForDeferredFlush()
            let payload = GaryxAvatarPayload(
                mediaType: entry.mediaType,
                data: data,
                contentFingerprint: entry.fingerprint
            )
            return GaryxStoredAvatar(record: entry, payload: payload)
        } catch {
            index.entries.removeValue(forKey: key)
            markIndexDirtyForDeferredFlush()
            return nil
        }
    }

    func avatarFingerprints(for identities: [GaryxAvatarIdentity], now: Date = Date()) async -> [GaryxAvatarIdentity: String] {
        await warm()
        var result: [GaryxAvatarIdentity: String] = [:]
        var changed = false
        for identity in identities {
            let key = identity.storageKey
            guard var entry = index.entries[key] else { continue }
            let url = directory.appendingPathComponent(entry.fileName, isDirectory: false)
            guard FileManager.default.fileExists(atPath: url.path) else {
                index.entries.removeValue(forKey: key)
                changed = true
                continue
            }
            entry.lastAccessAt = now
            index.entries[key] = entry
            result[identity] = entry.fingerprint
            changed = true
        }
        if changed { markIndexDirtyForDeferredFlush() }
        return result
    }

    @discardableResult
    func upsert(
        _ incoming: [GaryxAvatarUpsert],
        validator: any GaryxAvatarImageValidating = GaryxAvatarCGImageValidator(),
        now: Date = Date()
    ) async -> GaryxAvatarStoreWriteResult {
        await warm()
        guard !incoming.isEmpty else {
            return GaryxAvatarStoreWriteResult()
        }
        let current = Dictionary(uniqueKeysWithValues: index.entries.map { ($0.key, $0.value.fingerprint) })
        let plan = GaryxAvatarWriteThroughPlan.evaluate(
            incoming: incoming,
            currentFingerprints: current,
            validator: validator,
            policy: policy
        )
        var written = 0

        for item in plan.plannedUpserts {
            let key = item.identity.storageKey
            let fileName = item.identity.blobFileName
            let url = directory.appendingPathComponent(fileName, isDirectory: false)
            do {
                try item.payload.data.write(to: url, options: [.atomic])
                index.entries[key] = GaryxAvatarStoreEntry(
                    identity: item.identity,
                    fingerprint: item.payload.contentFingerprint,
                    fileName: fileName,
                    mediaType: item.payload.mediaType,
                    byteCount: item.payload.data.count,
                    sourceUpdatedAt: item.sourceUpdatedAt,
                    updatedAt: now,
                    lastAccessAt: now
                )
                written += 1
            } catch {
                continue
            }
        }
        await prune(policy: policy, now: now)
        writeIndexImmediately()
        return GaryxAvatarStoreWriteResult(
            written: written,
            unchanged: plan.unchangedCount,
            rejected: plan.rejectedCount
        )
    }

    func remove(_ identity: GaryxAvatarIdentity) async {
        await warm()
        guard let entry = index.entries.removeValue(forKey: identity.storageKey) else { return }
        let url = directory.appendingPathComponent(entry.fileName, isDirectory: false)
        try? FileManager.default.removeItem(at: url)
        writeIndexImmediately()
    }

    func prune(policy: GaryxAvatarPruningPolicy = .default, now: Date = Date()) async {
        await warm()
        var entries = index.entries
        var totalBytes = entries.values.reduce(0) { $0 + $1.byteCount }
        let sortedKeys = entries.values
            .sorted { lhs, rhs in
                if lhs.lastAccessAt == rhs.lastAccessAt {
                    return lhs.updatedAt < rhs.updatedAt
                }
                return lhs.lastAccessAt < rhs.lastAccessAt
            }
            .map { $0.identity.storageKey }
        var cursor = 0
        while entries.count > policy.maxRecords || totalBytes > policy.maxBytes {
            guard cursor < sortedKeys.count else { break }
            let key = sortedKeys[cursor]
            cursor += 1
            guard let removed = entries.removeValue(forKey: key) else { continue }
            totalBytes -= removed.byteCount
            let url = directory.appendingPathComponent(removed.fileName, isDirectory: false)
            try? FileManager.default.removeItem(at: url)
        }
        let didPrune = entries != index.entries
        index.entries = entries
        if didPrune {
            markIndexDirtyForDeferredFlush()
        }
    }

    func indexSnapshot() async -> GaryxAvatarStoreIndex {
        await warm()
        return index
    }

    private var indexURL: URL {
        directory.appendingPathComponent("index.json", isDirectory: false)
    }

    private func markIndexDirtyForDeferredFlush() {
        indexDirty = true
        scheduleDeferredIndexFlush()
    }

    private func scheduleDeferredIndexFlush() {
        guard deferredIndexFlushTask == nil else { return }
        deferredIndexFlushTask = Task {
            try? await Task.sleep(nanoseconds: 750_000_000)
            await flushDeferredIndexWrite()
        }
    }

    private func flushDeferredIndexWrite() {
        deferredIndexFlushTask = nil
        guard indexDirty else { return }
        writeIndexImmediately()
    }

    private func writeIndexImmediately() {
        deferredIndexFlushTask?.cancel()
        deferredIndexFlushTask = nil
        do {
            try writeIndex()
            indexDirty = false
        } catch {
            indexDirty = true
            scheduleDeferredIndexFlush()
        }
    }

    private func writeIndex() throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let encoder = JSONEncoder()
        let data = try encoder.encode(index)
        try data.write(to: indexURL, options: [.atomic])
    }

    private func excludeFromBackup(_ url: URL) throws {
        var resourceValues = URLResourceValues()
        resourceValues.isExcludedFromBackup = true
        var mutableURL = url
        try mutableURL.setResourceValues(resourceValues)
    }
}

struct GaryxAvatarRequestKey: Hashable {
    var identity: GaryxAvatarIdentity?
    var liveToken: String
}

struct GaryxAvatarRequest: Hashable {
    var identity: GaryxAvatarIdentity?
    var liveDataUrl: String

    var key: GaryxAvatarRequestKey {
        GaryxAvatarRequestKey(
            identity: identity,
            liveToken: GaryxAvatarFingerprint.rawStringToken(liveDataUrl)
        )
    }

    var remoteAvatarURL: URL? {
        let raw = liveDataUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        guard raw.hasPrefix("http://") || raw.hasPrefix("https://") else { return nil }
        return URL(string: raw)
    }
}

@MainActor
final class GaryxAvatarImageProvider {
    private let store: any GaryxAvatarStore
    private let validator: any GaryxAvatarImageValidating
    private var storedFingerprints: [GaryxAvatarIdentity: String] = [:]

    init(store: any GaryxAvatarStore, validator: any GaryxAvatarImageValidating) {
        self.store = store
        self.validator = validator
    }

    func syncImage(_ request: GaryxAvatarRequest) -> UIImage? {
        guard request.remoteAvatarURL == nil else { return nil }
        if let image = GaryxDataURLImageCache.cachedImage(
            from: request.liveDataUrl,
            maxPixelSize: GaryxDataURLImageCache.agentAvatarMaxPixelSize
        ) {
            return image
        }
        guard let identity = request.identity,
              let fingerprint = storedFingerprints[identity] else {
            return nil
        }
        return GaryxDataURLImageCache.cachedAvatarImage(
            identity: identity,
            fingerprint: fingerprint,
            maxPixelSize: GaryxDataURLImageCache.agentAvatarMaxPixelSize
        )
    }

    func resolve(_ request: GaryxAvatarRequest) async -> UIImage? {
        guard request.remoteAvatarURL == nil else { return nil }
        if let live = await decodeLiveImage(request.liveDataUrl) {
            return live
        }
        guard let identity = request.identity,
              let stored = await store.storedAvatar(for: identity, now: Date()) else {
            return nil
        }
        if let image = await decodeStoredAvatar(stored) {
            storedFingerprints[identity] = stored.record.fingerprint
            return image
        }
        return nil
    }

    func invalidate(identity: GaryxAvatarIdentity) {
        storedFingerprints.removeValue(forKey: identity)
    }

    private func decodeLiveImage(_ dataUrl: String) async -> UIImage? {
        let validator = validator
        return await Task.detached(priority: .utility) {
            guard let payload = GaryxAvatarDataURLParser.parse(dataUrl),
                  validator.validate(payload: payload) else {
                return nil
            }
            return GaryxDataURLImageCache.image(
                from: dataUrl,
                maxPixelSize: GaryxDataURLImageCache.agentAvatarMaxPixelSize
            )
        }.value
    }

    private func decodeStoredAvatar(_ stored: GaryxStoredAvatar) async -> UIImage? {
        await Task.detached(priority: .utility) {
            GaryxDataURLImageCache.image(
                from: stored.payload.data,
                cacheKey: GaryxDataURLImageCache.avatarCacheKey(
                    identity: stored.record.identity,
                    fingerprint: stored.record.fingerprint,
                    maxPixelSize: GaryxDataURLImageCache.agentAvatarMaxPixelSize
                ),
                maxPixelSize: GaryxDataURLImageCache.agentAvatarMaxPixelSize
            )
        }.value
    }
}

private struct GaryxAvatarImageProviderEnvironmentKey: EnvironmentKey {
    static let defaultValue: GaryxAvatarImageProvider? = nil
}

private struct GaryxAvatarScopeEnvironmentKey: EnvironmentKey {
    static let defaultValue = ""
}

extension EnvironmentValues {
    var garyxAvatarImageProvider: GaryxAvatarImageProvider? {
        get { self[GaryxAvatarImageProviderEnvironmentKey.self] }
        set { self[GaryxAvatarImageProviderEnvironmentKey.self] = newValue }
    }

    var garyxAvatarScopeId: String {
        get { self[GaryxAvatarScopeEnvironmentKey.self] }
        set { self[GaryxAvatarScopeEnvironmentKey.self] = newValue }
    }
}
