import Foundation

public struct GaryxMobileWidgetThread: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var title: String
    public var workspaceName: String
    public var updatedAt: String?
    public var activeRunId: String?
    public var runState: String?
    public var agentId: String?
    public var agentName: String?
    public var avatarDataUrl: String?
    public var avatarScope: String?
    public var avatarFingerprint: String?
    public var providerType: String?
    public var isTeam: Bool
    public var builtIn: Bool
    public var avatarPayloadData: Data?

    public init(
        id: String,
        title: String,
        workspaceName: String = "",
        updatedAt: String? = nil,
        activeRunId: String? = nil,
        runState: String? = nil,
        agentId: String? = nil,
        agentName: String? = nil,
        avatarDataUrl: String? = nil,
        avatarScope: String? = nil,
        avatarFingerprint: String? = nil,
        providerType: String? = nil,
        isTeam: Bool = false,
        builtIn: Bool = false,
        avatarPayloadData: Data? = nil
    ) {
        self.id = id
        self.title = title
        self.workspaceName = workspaceName
        self.updatedAt = updatedAt
        self.activeRunId = activeRunId
        self.runState = runState
        self.agentId = agentId
        self.agentName = agentName
        self.avatarDataUrl = avatarDataUrl
        self.avatarScope = avatarScope
        self.avatarFingerprint = avatarFingerprint
        self.providerType = providerType
        self.isTeam = isTeam
        self.builtIn = builtIn
        self.avatarPayloadData = avatarPayloadData
    }

    enum CodingKeys: String, CodingKey {
        case id
        case title
        case workspaceName
        case updatedAt
        case activeRunId
        case runState
        case agentId
        case agentName
        case avatarDataUrl
        case avatarScope
        case avatarFingerprint
        case providerType
        case isTeam
        case builtIn
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        title = try container.decode(String.self, forKey: .title)
        workspaceName = try container.decodeIfPresent(String.self, forKey: .workspaceName) ?? ""
        updatedAt = try container.decodeIfPresent(String.self, forKey: .updatedAt)
        activeRunId = try container.decodeIfPresent(String.self, forKey: .activeRunId)
        runState = try container.decodeIfPresent(String.self, forKey: .runState)
        agentId = try container.decodeIfPresent(String.self, forKey: .agentId)
        agentName = try container.decodeIfPresent(String.self, forKey: .agentName)
        avatarDataUrl = try container.decodeIfPresent(String.self, forKey: .avatarDataUrl)
        avatarScope = try container.decodeIfPresent(String.self, forKey: .avatarScope)
        avatarFingerprint = try container.decodeIfPresent(String.self, forKey: .avatarFingerprint)
        providerType = try container.decodeIfPresent(String.self, forKey: .providerType)
        isTeam = try container.decodeIfPresent(Bool.self, forKey: .isTeam) ?? false
        builtIn = try container.decodeIfPresent(Bool.self, forKey: .builtIn) ?? false
        avatarPayloadData = nil
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(title, forKey: .title)
        try container.encode(workspaceName, forKey: .workspaceName)
        try container.encodeIfPresent(updatedAt, forKey: .updatedAt)
        try container.encodeIfPresent(activeRunId, forKey: .activeRunId)
        try container.encodeIfPresent(runState, forKey: .runState)
        try container.encodeIfPresent(agentId, forKey: .agentId)
        try container.encodeIfPresent(agentName, forKey: .agentName)
        try container.encodeIfPresent(avatarDataUrl, forKey: .avatarDataUrl)
        try container.encodeIfPresent(avatarScope, forKey: .avatarScope)
        try container.encodeIfPresent(avatarFingerprint, forKey: .avatarFingerprint)
        try container.encodeIfPresent(providerType, forKey: .providerType)
        try container.encode(isTeam, forKey: .isTeam)
        try container.encode(builtIn, forKey: .builtIn)
    }
}

public struct GaryxMobileWidgetSnapshot: Codable, Equatable, Sendable {
    public var threads: [GaryxMobileWidgetThread]
    public var refreshedAt: Date

    public init(threads: [GaryxMobileWidgetThread], refreshedAt: Date = Date()) {
        self.threads = Array(threads.prefix(GaryxMobileWidgetStore.storedThreadLimit))
        self.refreshedAt = refreshedAt
    }

    public static let empty = GaryxMobileWidgetSnapshot(threads: [], refreshedAt: .distantPast)
}

public enum GaryxMobileWidgetStore {
    public static let appGroupIdentifier = "group.com.garyx.mobile"
    public static let visibleThreadLimit = 5
    public static let storedThreadLimit = 20
    public static let threadLimit = visibleThreadLimit

    private static let recentThreadsKey = "garyx.mobile.widget.recentThreads"

    public static func sharedDefaults() -> UserDefaults {
        UserDefaults(suiteName: appGroupIdentifier) ?? .standard
    }

    public static func saveRecentThreads(
        _ threads: [GaryxMobileWidgetThread],
        refreshedAt: Date = Date(),
        defaults: UserDefaults = sharedDefaults()
    ) {
        let snapshot = GaryxMobileWidgetSnapshot(threads: threads, refreshedAt: refreshedAt)
        guard let data = try? JSONEncoder().encode(snapshot) else { return }
        defaults.set(data, forKey: recentThreadsKey)
    }

    public static func loadRecentThreads(
        defaults: UserDefaults = sharedDefaults()
    ) -> GaryxMobileWidgetSnapshot {
        guard let data = defaults.data(forKey: recentThreadsKey),
              let snapshot = try? JSONDecoder().decode(GaryxMobileWidgetSnapshot.self, from: data) else {
            return .empty
        }
        return GaryxMobileWidgetSnapshot(threads: snapshot.threads, refreshedAt: snapshot.refreshedAt)
    }

    public static func clear(defaults: UserDefaults = sharedDefaults()) {
        defaults.removeObject(forKey: recentThreadsKey)
    }
}

public enum GaryxRecentThreadsWidgetConstants {
    public static let kind = "GaryxRecentThreadsWidget"
}

public enum GaryxMobileThreadLink {
    public static func make(threadId: String) -> URL? {
        let normalizedId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return nil }

        var components = URLComponents()
        components.scheme = "garyx"
        components.host = "mobile"
        components.path = "/thread"
        components.queryItems = [
            URLQueryItem(name: "threadId", value: normalizedId),
        ]
        return components.url
    }

    public static func parse(_ url: URL) -> String? {
        guard url.scheme?.lowercased() == "garyx" else { return nil }
        let host = url.host()?.lowercased()
        let path = url.path.trimmingCharacters(in: CharacterSet(charactersIn: "/")).lowercased()
        guard host == "thread" || path == "thread" || path == "mobile/thread" else {
            return nil
        }
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            return nil
        }
        for name in ["threadId", "thread_id", "id"] {
            if let value = components.queryItems?
                .first(where: { $0.name == name })?
                .value?
                .trimmingCharacters(in: .whitespacesAndNewlines),
                !value.isEmpty {
                return value
            }
        }
        return nil
    }
}
