import Foundation

public enum GaryxAvatarKind: String, Codable, Equatable, Sendable {
    case agent
    case team
}

public struct GaryxAvatarIdentity: Hashable, Codable, Sendable {
    public var scope: String
    public var kind: GaryxAvatarKind
    public var id: String

    public init(scope: String, kind: GaryxAvatarKind, id: String) {
        self.scope = scope.trimmingCharacters(in: .whitespacesAndNewlines)
        self.kind = kind
        self.id = id.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    public var storageKey: String {
        "\(scope)|\(kind.rawValue)|\(id)"
    }

    public var isUsable: Bool {
        !scope.isEmpty && !id.isEmpty
    }

    public var blobFileName: String {
        "avatar-\(GaryxAvatarFingerprint.fnv1a64Hex(storageKey.utf8)).bin"
    }
}

public struct GaryxAvatarPayload: Equatable, Sendable {
    public var mediaType: String
    public var data: Data
    public var contentFingerprint: String

    public init(mediaType: String, data: Data, contentFingerprint: String? = nil) {
        self.mediaType = mediaType.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        self.data = data
        self.contentFingerprint = contentFingerprint ?? GaryxAvatarFingerprint.contentFingerprint(for: data)
    }

    public var dataUrlString: String {
        "data:\(mediaType);base64,\(data.base64EncodedString())"
    }
}

public struct GaryxStoredAvatar: Equatable, Sendable {
    public var record: GaryxAvatarStoreEntry
    public var payload: GaryxAvatarPayload

    public init(record: GaryxAvatarStoreEntry, payload: GaryxAvatarPayload) {
        self.record = record
        self.payload = payload
    }
}

public struct GaryxAvatarUpsert: Equatable, Sendable {
    public var identity: GaryxAvatarIdentity
    public var dataUrl: String
    public var sourceUpdatedAt: String?

    public init(identity: GaryxAvatarIdentity, dataUrl: String, sourceUpdatedAt: String? = nil) {
        self.identity = identity
        self.dataUrl = dataUrl
        self.sourceUpdatedAt = sourceUpdatedAt?.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

public struct GaryxAvatarPlannedUpsert: Equatable, Sendable {
    public var identity: GaryxAvatarIdentity
    public var payload: GaryxAvatarPayload
    public var sourceUpdatedAt: String?

    public init(identity: GaryxAvatarIdentity, payload: GaryxAvatarPayload, sourceUpdatedAt: String? = nil) {
        self.identity = identity
        self.payload = payload
        self.sourceUpdatedAt = sourceUpdatedAt
    }
}

public struct GaryxAvatarStoreEntry: Codable, Equatable, Sendable {
    public var identity: GaryxAvatarIdentity
    public var fingerprint: String
    public var fileName: String
    public var mediaType: String
    public var byteCount: Int
    public var sourceUpdatedAt: String?
    public var updatedAt: Date
    public var lastAccessAt: Date

    public init(
        identity: GaryxAvatarIdentity,
        fingerprint: String,
        fileName: String,
        mediaType: String,
        byteCount: Int,
        sourceUpdatedAt: String? = nil,
        updatedAt: Date,
        lastAccessAt: Date
    ) {
        self.identity = identity
        self.fingerprint = fingerprint
        self.fileName = fileName
        self.mediaType = mediaType
        self.byteCount = byteCount
        self.sourceUpdatedAt = sourceUpdatedAt
        self.updatedAt = updatedAt
        self.lastAccessAt = lastAccessAt
    }
}

public struct GaryxAvatarStoreIndex: Codable, Equatable, Sendable {
    public static let currentVersion = 1

    public var version: Int
    public var entries: [String: GaryxAvatarStoreEntry]

    public init(version: Int = Self.currentVersion, entries: [String: GaryxAvatarStoreEntry] = [:]) {
        self.version = version
        self.entries = entries
    }

    public var isCurrentVersion: Bool {
        version == Self.currentVersion
    }

    public static func decodeCurrent(from data: Data, decoder: JSONDecoder = JSONDecoder()) -> GaryxAvatarStoreIndex? {
        guard let index = try? decoder.decode(GaryxAvatarStoreIndex.self, from: data),
              index.version == Self.currentVersion else {
            return nil
        }
        return index
    }
}

public struct GaryxAvatarPruningPolicy: Equatable, Sendable {
    public static let `default` = GaryxAvatarPruningPolicy(maxRecords: 256, maxBytes: 16 * 1024 * 1024, maxRecordBytes: 512 * 1024)

    public var maxRecords: Int
    public var maxBytes: Int
    public var maxRecordBytes: Int

    public init(maxRecords: Int, maxBytes: Int, maxRecordBytes: Int) {
        self.maxRecords = max(0, maxRecords)
        self.maxBytes = max(0, maxBytes)
        self.maxRecordBytes = max(0, maxRecordBytes)
    }
}

public enum GaryxResolvedAvatarSource: Equatable, Sendable {
    case live(GaryxAvatarPayload)
    case stored(GaryxStoredAvatar)
    case placeholder
}

public enum GaryxAvatarResolution {
    public static func resolve(live: GaryxAvatarPayload?, stored: GaryxStoredAvatar?) -> GaryxResolvedAvatarSource {
        if let live {
            return .live(live)
        }
        if let stored {
            return .stored(stored)
        }
        return .placeholder
    }
}

public protocol GaryxAvatarImageValidating: Sendable {
    func validate(payload: GaryxAvatarPayload) -> Bool
}

public struct GaryxAvatarAlwaysValidImageValidator: GaryxAvatarImageValidating {
    public init() {}
    public func validate(payload: GaryxAvatarPayload) -> Bool {
        true
    }
}

public struct GaryxAvatarNeverValidImageValidator: GaryxAvatarImageValidating {
    public init() {}
    public func validate(payload: GaryxAvatarPayload) -> Bool {
        false
    }
}

public enum GaryxAvatarDataURLParser {
    public static func parse(_ rawValue: String?, policy: GaryxAvatarPruningPolicy = .default) -> GaryxAvatarPayload? {
        let raw = (rawValue ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty,
              !raw.lowercased().hasPrefix("http://"),
              !raw.lowercased().hasPrefix("https://") else {
            return nil
        }

        let parts = raw.split(separator: ",", maxSplits: 1, omittingEmptySubsequences: false)
        guard parts.count == 2 else { return nil }
        let header = parts[0].trimmingCharacters(in: .whitespacesAndNewlines)
        let encoded = String(parts[1])
        guard header.lowercased().hasPrefix("data:") else { return nil }

        let descriptor = header.dropFirst("data:".count)
        let components = descriptor.split(separator: ";", omittingEmptySubsequences: false)
        guard let mediaComponent = components.first else { return nil }
        let mediaType = String(mediaComponent).trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard mediaType.hasPrefix("image/"),
              components.dropFirst().contains(where: { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() == "base64" }) else {
            return nil
        }
        let cleanedEncoded = encoded.filter { !$0.isWhitespace }
        guard cleanedEncoded.allSatisfy(Self.isStandardBase64Character),
              let data = Data(base64Encoded: String(cleanedEncoded)),
              !data.isEmpty,
              data.count <= policy.maxRecordBytes else {
            return nil
        }
        return GaryxAvatarPayload(mediaType: mediaType, data: data)
    }

    private static func isStandardBase64Character(_ character: Character) -> Bool {
        guard character.unicodeScalars.count == 1,
              let scalar = character.unicodeScalars.first else {
            return false
        }
        switch scalar.value {
        case 65...90, 97...122, 48...57:
            return true
        case 43, 47, 61:
            return true
        default:
            return false
        }
    }
}

public enum GaryxAvatarFingerprint {
    public static func contentFingerprint(for data: Data) -> String {
        "fnv1a64:\(fnv1a64Hex(data))"
    }

    public static func rawStringToken(_ value: String) -> String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "" }
        return "\(trimmed.utf8.count):\(fnv1a64Hex(trimmed.utf8))"
    }

    public static func fnv1a64Hex<S: Sequence>(_ bytes: S) -> String where S.Element == UInt8 {
        let value = fnv1a64(bytes)
        return String(format: "%016llx", value)
    }

    public static func fnv1a64<S: Sequence>(_ bytes: S) -> UInt64 where S.Element == UInt8 {
        var hash: UInt64 = 14_695_981_039_346_656_037
        for byte in bytes {
            hash ^= UInt64(byte)
            hash = hash &* 1_099_511_628_211
        }
        return hash
    }
}

public enum GaryxAvatarWriteThroughPlan {
    public struct Evaluation: Equatable, Sendable {
        public var plannedUpserts: [GaryxAvatarPlannedUpsert]
        public var unchangedCount: Int
        public var rejectedCount: Int

        public init(
            plannedUpserts: [GaryxAvatarPlannedUpsert],
            unchangedCount: Int,
            rejectedCount: Int
        ) {
            self.plannedUpserts = plannedUpserts
            self.unchangedCount = unchangedCount
            self.rejectedCount = rejectedCount
        }
    }

    public static func upserts(
        incoming: [GaryxAvatarUpsert],
        currentFingerprints: [String: String],
        validator: any GaryxAvatarImageValidating,
        policy: GaryxAvatarPruningPolicy = .default
    ) -> [GaryxAvatarPlannedUpsert] {
        evaluate(
            incoming: incoming,
            currentFingerprints: currentFingerprints,
            validator: validator,
            policy: policy
        ).plannedUpserts
    }

    public static func evaluate(
        incoming: [GaryxAvatarUpsert],
        currentFingerprints: [String: String],
        validator: any GaryxAvatarImageValidating,
        policy: GaryxAvatarPruningPolicy = .default
    ) -> Evaluation {
        var planned: [GaryxAvatarPlannedUpsert] = []
        var unchanged = 0
        var rejected = 0
        var seen = Set<String>()
        for item in incoming {
            guard item.identity.isUsable,
                  !item.dataUrl.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  let payload = GaryxAvatarDataURLParser.parse(item.dataUrl, policy: policy),
                  validator.validate(payload: payload) else {
                rejected += 1
                continue
            }
            let key = item.identity.storageKey
            guard seen.insert(key).inserted else {
                unchanged += 1
                continue
            }
            guard currentFingerprints[key] != payload.contentFingerprint else {
                unchanged += 1
                continue
            }
            planned.append(GaryxAvatarPlannedUpsert(identity: item.identity, payload: payload, sourceUpdatedAt: item.sourceUpdatedAt))
        }
        return Evaluation(
            plannedUpserts: planned,
            unchangedCount: unchanged,
            rejectedCount: rejected
        )
    }

}

public struct GaryxAvatarStoreWriteResult: Equatable, Sendable {
    public var written: Int
    public var unchanged: Int
    public var rejected: Int

    public init(written: Int = 0, unchanged: Int = 0, rejected: Int = 0) {
        self.written = written
        self.unchanged = unchanged
        self.rejected = rejected
    }
}

public protocol GaryxAvatarStore: Sendable {
    func warm() async
    func storedAvatar(for identity: GaryxAvatarIdentity, now: Date) async -> GaryxStoredAvatar?
    func avatarFingerprints(for identities: [GaryxAvatarIdentity], now: Date) async -> [GaryxAvatarIdentity: String]
    @discardableResult
    func upsert(
        _ incoming: [GaryxAvatarUpsert],
        validator: any GaryxAvatarImageValidating,
        now: Date
    ) async -> GaryxAvatarStoreWriteResult
    func remove(_ identity: GaryxAvatarIdentity) async
    func prune(policy: GaryxAvatarPruningPolicy, now: Date) async
    func indexSnapshot() async -> GaryxAvatarStoreIndex
}

public actor GaryxInMemoryAvatarStore: GaryxAvatarStore {
    private var index: GaryxAvatarStoreIndex
    private var payloads: [String: GaryxAvatarPayload]
    public private(set) var payloadWriteCount: Int

    public init(index: GaryxAvatarStoreIndex = GaryxAvatarStoreIndex()) {
        self.index = index.isCurrentVersion ? index : GaryxAvatarStoreIndex()
        self.payloads = [:]
        self.payloadWriteCount = 0
    }

    public func warm() async {}

    public func storedAvatar(for identity: GaryxAvatarIdentity, now: Date = Date()) async -> GaryxStoredAvatar? {
        let key = identity.storageKey
        guard var entry = index.entries[key],
              let payload = payloads[key] else {
            return nil
        }
        entry.lastAccessAt = now
        index.entries[key] = entry
        return GaryxStoredAvatar(record: entry, payload: payload)
    }

    public func avatarFingerprints(for identities: [GaryxAvatarIdentity], now: Date = Date()) async -> [GaryxAvatarIdentity: String] {
        var result: [GaryxAvatarIdentity: String] = [:]
        for identity in identities {
            let key = identity.storageKey
            guard var entry = index.entries[key],
                  payloads[key] != nil else {
                continue
            }
            entry.lastAccessAt = now
            index.entries[key] = entry
            result[identity] = entry.fingerprint
        }
        return result
    }

    @discardableResult
    public func upsert(
        _ incoming: [GaryxAvatarUpsert],
        validator: any GaryxAvatarImageValidating = GaryxAvatarAlwaysValidImageValidator(),
        now: Date = Date()
    ) async -> GaryxAvatarStoreWriteResult {
        let current = Dictionary(uniqueKeysWithValues: index.entries.map { ($0.key, $0.value.fingerprint) })
        let plan = GaryxAvatarWriteThroughPlan.evaluate(
            incoming: incoming,
            currentFingerprints: current,
            validator: validator
        )

        for item in plan.plannedUpserts {
            let key = item.identity.storageKey
            payloads[key] = item.payload
            index.entries[key] = GaryxAvatarStoreEntry(
                identity: item.identity,
                fingerprint: item.payload.contentFingerprint,
                fileName: item.identity.blobFileName,
                mediaType: item.payload.mediaType,
                byteCount: item.payload.data.count,
                sourceUpdatedAt: item.sourceUpdatedAt,
                updatedAt: now,
                lastAccessAt: now
            )
            payloadWriteCount += 1
        }
        await prune(policy: .default, now: now)
        return GaryxAvatarStoreWriteResult(
            written: plan.plannedUpserts.count,
            unchanged: plan.unchangedCount,
            rejected: plan.rejectedCount
        )
    }

    public func remove(_ identity: GaryxAvatarIdentity) async {
        let key = identity.storageKey
        index.entries.removeValue(forKey: key)
        payloads.removeValue(forKey: key)
    }

    public func prune(policy: GaryxAvatarPruningPolicy = .default, now: Date = Date()) async {
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
            if let removed = entries.removeValue(forKey: key) {
                totalBytes -= removed.byteCount
                payloads.removeValue(forKey: key)
            }
        }
        index.entries = entries
    }

    public func indexSnapshot() async -> GaryxAvatarStoreIndex {
        index
    }
}
