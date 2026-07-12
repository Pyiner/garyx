import Foundation

public enum GaryxRecentThreadFilterStorage {
    public static func persistenceValue(for filter: GaryxRecentThreadFilter) -> String {
        switch filter {
        case .all: return "all"
        case .nonTask: return "nonTask"
        }
    }

    public static func restoredFilter(from rawValue: String?) -> GaryxRecentThreadFilter {
        switch rawValue {
        case "all": return .all
        case "nonTask": return .nonTask
        default: return .all
        }
    }

    public static func load(
        defaults: UserDefaults,
        key: String
    ) -> GaryxRecentThreadFilter {
        restoredFilter(from: defaults.string(forKey: key))
    }

    public static func save(
        _ filter: GaryxRecentThreadFilter,
        defaults: UserDefaults,
        key: String
    ) {
        defaults.set(persistenceValue(for: filter), forKey: key)
    }
}
