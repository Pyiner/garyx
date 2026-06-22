import Foundation

/// Compact relative-time label ("now"/"3m"/"5h"/"2d"/"4mo"/"1y") used across
/// sidebar/task/home lists. `now` is injectable so callers that must refresh
/// on a clock tick (and tests) can drive the reference instant; production
/// call sites use the wall clock by default.
///
/// Lives in `GaryxMobileCore` (pure formatting/presentation logic) so the
/// production implementation is what `swift test` exercises — not an App-target
/// copy. In the app Xcode target these Core sources compile into the same
/// `GaryxMobile` module, so App call sites use it without importing Core.
func garyxFormattedTaskTimestamp(_ value: String?, now: Date = Date()) -> String {
    guard let value, let date = garyxISO8601Date(from: value) else {
        return ""
    }
    let diff = max(0, now.timeIntervalSince(date))
    let minutes = Int(diff / 60)
    let hours = Int(diff / 3_600)
    let days = Int(diff / 86_400)
    let months = days / 30
    if minutes < 1 { return "now" }
    if minutes < 60 { return "\(minutes)m" }
    if hours < 24 { return "\(hours)h" }
    if days < 30 { return "\(days)d" }
    if months < 12 { return "\(months)mo" }
    return "\(days / 365)y"
}

func garyxThreadDate(from value: String) -> Date? {
    garyxISO8601Date(from: value)
}

// Formatter construction is expensive and these run per row per render in
// sidebar/task lists, so keep shared instances (ISO8601DateFormatter is
// thread-safe) plus a bounded parse cache keyed by the raw timestamp string.
private let garyxISO8601FractionalFormatter: ISO8601DateFormatter = {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return formatter
}()

private let garyxISO8601StandardFormatter: ISO8601DateFormatter = {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime]
    return formatter
}()

private let garyxISO8601DateCache: NSCache<NSString, NSDate> = {
    let cache = NSCache<NSString, NSDate>()
    cache.countLimit = 4096
    return cache
}()

private func garyxISO8601Date(from value: String) -> Date? {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return nil }

    let cacheKey = trimmed as NSString
    if let cached = garyxISO8601DateCache.object(forKey: cacheKey) {
        return cached as Date
    }
    let parsed = garyxISO8601FractionalFormatter.date(from: trimmed)
        ?? garyxISO8601StandardFormatter.date(from: trimmed)
    if let parsed {
        garyxISO8601DateCache.setObject(parsed as NSDate, forKey: cacheKey)
    }
    return parsed
}
