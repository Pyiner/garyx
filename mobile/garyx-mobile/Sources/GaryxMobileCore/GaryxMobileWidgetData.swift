import Foundation

public struct GaryxMobileWidgetThread: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var title: String
    public var workspaceName: String
    public var updatedAt: String?
    public var activeRunId: String?
    public var runState: String?

    public init(
        id: String,
        title: String,
        workspaceName: String = "",
        updatedAt: String? = nil,
        activeRunId: String? = nil,
        runState: String? = nil
    ) {
        self.id = id
        self.title = title
        self.workspaceName = workspaceName
        self.updatedAt = updatedAt
        self.activeRunId = activeRunId
        self.runState = runState
    }
}

public struct GaryxMobileWidgetSnapshot: Codable, Equatable, Sendable {
    public var threads: [GaryxMobileWidgetThread]
    public var refreshedAt: Date

    public init(threads: [GaryxMobileWidgetThread], refreshedAt: Date = Date()) {
        self.threads = Array(threads.prefix(GaryxMobileWidgetStore.threadLimit))
        self.refreshedAt = refreshedAt
    }

    public static let empty = GaryxMobileWidgetSnapshot(threads: [], refreshedAt: .distantPast)
}

public enum GaryxMobileWidgetStore {
    public static let appGroupIdentifier = "group.com.garyx.mobile"
    public static let threadLimit = 5

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
