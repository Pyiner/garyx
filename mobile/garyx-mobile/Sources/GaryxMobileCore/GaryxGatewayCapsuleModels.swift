import Foundation

public struct GaryxCapsulesPage: Decodable, Equatable, Sendable {
    public var capsules: [GaryxCapsuleSummary]

    public init(capsules: [GaryxCapsuleSummary] = []) {
        self.capsules = capsules
    }

    enum CodingKeys: String, CodingKey {
        case capsules
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        capsules = try container.decodeIfPresent([GaryxCapsuleSummary].self, forKey: .capsules) ?? []
    }
}

public struct GaryxCapsuleSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var title: String
    public var description: String
    public var threadId: String?
    public var runId: String?
    public var agentId: String?
    public var providerType: String?
    public var htmlSha256: String
    public var byteSize: Int
    public var revision: Int
    public var createdAt: String?
    public var updatedAt: String?

    public init(
        id: String,
        title: String,
        description: String = "",
        threadId: String? = nil,
        runId: String? = nil,
        agentId: String? = nil,
        providerType: String? = nil,
        htmlSha256: String = "",
        byteSize: Int = 0,
        revision: Int = 1,
        createdAt: String? = nil,
        updatedAt: String? = nil
    ) {
        self.id = id
        self.title = title
        self.description = description
        self.threadId = threadId
        self.runId = runId
        self.agentId = agentId
        self.providerType = providerType
        self.htmlSha256 = htmlSha256
        self.byteSize = byteSize
        self.revision = revision
        self.createdAt = createdAt
        self.updatedAt = updatedAt
    }

    enum CodingKeys: String, CodingKey {
        case id
        case title
        case description
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case runId = "run_id"
        case runIdCamel = "runId"
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
        case providerType = "provider_type"
        case providerTypeCamel = "providerType"
        case htmlSha256 = "html_sha256"
        case htmlSha256Camel = "htmlSha256"
        case byteSize = "byte_size"
        case byteSizeCamel = "byteSize"
        case revision
        case createdAt = "created_at"
        case createdAtCamel = "createdAt"
        case updatedAt = "updated_at"
        case updatedAtCamel = "updatedAt"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.garyxDecodeFirstString(.id) ?? ""
        title = try container.garyxDecodeFirstString(.title) ?? "Untitled Capsule"
        description = try container.garyxDecodeFirstString(.description) ?? ""
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel)
        runId = try container.garyxDecodeFirstString(.runId, .runIdCamel)
        agentId = try container.garyxDecodeFirstString(.agentId, .agentIdCamel)
        providerType = try container.garyxDecodeFirstString(.providerType, .providerTypeCamel)
        htmlSha256 = try container.garyxDecodeFirstString(.htmlSha256, .htmlSha256Camel) ?? ""
        byteSize = try container.garyxDecodeFirstInt(.byteSize, .byteSizeCamel) ?? 0
        revision = try container.garyxDecodeFirstInt(.revision) ?? 1
        createdAt = try container.garyxDecodeFirstString(.createdAt, .createdAtCamel)
        updatedAt = try container.garyxDecodeFirstString(.updatedAt, .updatedAtCamel)
    }
}

/// Shared preview-HTML cache key. Deliberately `(id, revision)` only — chat
/// cards carry no `html_sha256`, so including the sha would split the cache into
/// `id:rev:sha` (gallery/focused) and `id:rev` (chat) and double-fetch the same
/// capsule. `update_capsule` always bumps `revision` (even metadata-only), so
/// `(id, revision)` is a conservative invalidation key: it may over-invalidate a
/// metadata-only update by one refetch, but never serves stale content.
public struct GaryxCapsuleHTMLCacheKey: Hashable, Equatable, Sendable {
    public var id: String
    public var revision: Int

    public init(id: String, revision: Int) {
        self.id = id.trimmingCharacters(in: .whitespacesAndNewlines)
        self.revision = revision
    }

    public init(capsule: GaryxCapsuleSummary) {
        self.init(id: capsule.id, revision: capsule.revision)
    }
}

/// Pure prune of the preview-HTML cache against the authoritative capsules list.
/// Extracted from the model so the "did anything get evicted" signal that drives
/// the cache epoch is headless-testable. A deleted capsule (id absent from the
/// list) or a superseded revision drops out of the valid key set and is evicted.
public enum GaryxCapsuleHTMLCachePruner {
    public static func pruned(
        cache: [GaryxCapsuleHTMLCacheKey: String],
        validCapsules: [GaryxCapsuleSummary]
    ) -> (cache: [GaryxCapsuleHTMLCacheKey: String], didEvict: Bool) {
        let validKeys = Set(validCapsules.map(GaryxCapsuleHTMLCacheKey.init))
        let next = cache.filter { validKeys.contains($0.key) }
        return (next, next.count != cache.count)
    }
}
