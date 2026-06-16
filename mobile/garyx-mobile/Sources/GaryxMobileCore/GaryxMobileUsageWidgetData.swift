import Foundation

// MARK: - Wire + storage models

/// One usage window (weekly or session) as reported by the gateway
/// `GET /api/usage/coding` endpoint.
public struct GaryxUsageWindow: Codable, Equatable, Sendable {
    /// Percentage of the allowance already consumed (0-100).
    public var usedPercent: Double
    /// Percentage of the allowance still available (0-100).
    public var remainingPercent: Double
    /// ISO 8601 timestamp when the window resets, when known.
    public var resetsAt: String?
    /// Seconds until the window resets, when known.
    public var resetAfterSeconds: Int?

    public init(
        usedPercent: Double,
        remainingPercent: Double,
        resetsAt: String? = nil,
        resetAfterSeconds: Int? = nil
    ) {
        self.usedPercent = usedPercent
        self.remainingPercent = remainingPercent
        self.resetsAt = resetsAt
        self.resetAfterSeconds = resetAfterSeconds
    }

    enum CodingKeys: String, CodingKey {
        case usedPercent = "used_percent"
        case remainingPercent = "remaining_percent"
        case resetsAt = "resets_at"
        case resetAfterSeconds = "reset_after_seconds"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        usedPercent = try container.decodeIfPresent(Double.self, forKey: .usedPercent) ?? 0
        remainingPercent = try container.decodeIfPresent(Double.self, forKey: .remainingPercent)
            ?? max(0, 100 - usedPercent)
        resetsAt = try container.decodeIfPresent(String.self, forKey: .resetsAt)
        resetAfterSeconds = try container.decodeIfPresent(Int.self, forKey: .resetAfterSeconds)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(usedPercent, forKey: .usedPercent)
        try container.encode(remainingPercent, forKey: .remainingPercent)
        try container.encodeIfPresent(resetsAt, forKey: .resetsAt)
        try container.encodeIfPresent(resetAfterSeconds, forKey: .resetAfterSeconds)
    }
}

/// Usage for a single coding assistant on the gateway machine.
public struct GaryxProviderUsage: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var name: String
    public var available: Bool
    public var stale: Bool
    public var plan: String?
    public var weekly: GaryxUsageWindow?
    public var session: GaryxUsageWindow?
    public var error: String?

    public init(
        id: String,
        name: String,
        available: Bool,
        stale: Bool = false,
        plan: String? = nil,
        weekly: GaryxUsageWindow? = nil,
        session: GaryxUsageWindow? = nil,
        error: String? = nil
    ) {
        self.id = id
        self.name = name
        self.available = available
        self.stale = stale
        self.plan = plan
        self.weekly = weekly
        self.session = session
        self.error = error
    }

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case available
        case stale
        case plan
        case weekly
        case session
        case error
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        available = try container.decodeIfPresent(Bool.self, forKey: .available) ?? false
        stale = try container.decodeIfPresent(Bool.self, forKey: .stale) ?? false
        plan = try container.decodeIfPresent(String.self, forKey: .plan)
        weekly = try container.decodeIfPresent(GaryxUsageWindow.self, forKey: .weekly)
        session = try container.decodeIfPresent(GaryxUsageWindow.self, forKey: .session)
        error = try container.decodeIfPresent(String.self, forKey: .error)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(name, forKey: .name)
        try container.encode(available, forKey: .available)
        try container.encode(stale, forKey: .stale)
        try container.encodeIfPresent(plan, forKey: .plan)
        try container.encodeIfPresent(weekly, forKey: .weekly)
        try container.encodeIfPresent(session, forKey: .session)
        try container.encodeIfPresent(error, forKey: .error)
    }
}

/// Aggregate `GET /api/usage/coding` payload.
public struct GaryxCodingUsage: Codable, Equatable, Sendable {
    public var providers: [GaryxProviderUsage]
    public var refreshedAt: String?

    public init(providers: [GaryxProviderUsage], refreshedAt: String? = nil) {
        self.providers = providers
        self.refreshedAt = refreshedAt
    }

    enum CodingKeys: String, CodingKey {
        case providers
        case refreshedAt = "refreshed_at"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        providers = try container.decodeIfPresent([GaryxProviderUsage].self, forKey: .providers) ?? []
        refreshedAt = try container.decodeIfPresent(String.self, forKey: .refreshedAt)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(providers, forKey: .providers)
        try container.encodeIfPresent(refreshedAt, forKey: .refreshedAt)
    }

    /// Look up a provider by its stable id (`claude_code`, `codex`).
    public func provider(id: String) -> GaryxProviderUsage? {
        providers.first { $0.id == id }
    }
}

/// Snapshot persisted to the shared App Group so the widget can render the last
/// known reading even before its own fetch completes.
public struct GaryxUsageWidgetSnapshot: Codable, Equatable, Sendable {
    public var usage: GaryxCodingUsage
    public var fetchedAt: Date
    /// Set when the most recent refresh failed but a prior reading is shown.
    public init(usage: GaryxCodingUsage, fetchedAt: Date = Date()) {
        self.usage = usage
        self.fetchedAt = fetchedAt
    }

    public static let empty = GaryxUsageWidgetSnapshot(
        usage: GaryxCodingUsage(providers: []),
        fetchedAt: .distantPast
    )

    public var isEmpty: Bool { usage.providers.isEmpty }

    /// Human "updated Nm ago" label derived from `fetchedAt` so a stale,
    /// app-warmed snapshot never reads as if it just refreshed. Returns nil when
    /// the snapshot was never fetched.
    public func ageText(asOf now: Date = Date()) -> String? {
        guard fetchedAt > .distantPast else { return nil }
        let seconds = Int(now.timeIntervalSince(fetchedAt).rounded())
        if seconds < 60 { return "updated just now" }
        return "updated \(GaryxUsageGaugeModel.formatDuration(seconds)) ago"
    }
}

// MARK: - Shared store
//
// The widget reads an app-warmed snapshot from the shared App Group; it does NOT
// fetch the gateway itself and no gateway URL or auth token is stored in shared
// defaults. The app owns the network fetch (authenticated via Keychain) and
// writes only the numeric usage snapshot here.

public enum GaryxUsageWidgetStore {
    private static let snapshotKey = "garyx.mobile.widget.codingUsage"

    public static func sharedDefaults() -> UserDefaults {
        GaryxMobileWidgetStore.sharedDefaults()
    }

    public static func saveSnapshot(
        _ snapshot: GaryxUsageWidgetSnapshot,
        defaults: UserDefaults = sharedDefaults()
    ) {
        guard let data = try? JSONEncoder().encode(snapshot) else { return }
        defaults.set(data, forKey: snapshotKey)
    }

    public static func loadSnapshot(
        defaults: UserDefaults = sharedDefaults()
    ) -> GaryxUsageWidgetSnapshot {
        guard let data = defaults.data(forKey: snapshotKey),
              let snapshot = try? JSONDecoder().decode(GaryxUsageWidgetSnapshot.self, from: data) else {
            return .empty
        }
        return snapshot
    }

    public static func clear(defaults: UserDefaults = sharedDefaults()) {
        defaults.removeObject(forKey: snapshotKey)
    }
}

// MARK: - Gauge presentation (testable business rules)

/// How healthy a remaining-quota reading is, driving the gauge colour.
public enum GaryxUsageLevel: String, Equatable, Sendable {
    case healthy
    case warning
    case critical
    case unavailable
}

/// View-ready model for a single speedometer gauge. All formatting and
/// thresholds live here (per the Core-owns-business-rules guideline) so the
/// widget view stays a pure renderer.
public struct GaryxUsageGaugeModel: Equatable, Sendable {
    public var providerId: String
    public var title: String
    /// Fraction the gauge fills, 0...1, representing remaining weekly quota.
    public var fillFraction: Double
    /// Big readout, e.g. `73%` or `—`.
    public var remainingText: String
    /// Secondary line, e.g. `resets in 2d` / `weekly limit` / failure reason.
    public var detailText: String
    public var level: GaryxUsageLevel
    public var available: Bool
    /// SF Symbol resolved through the shared provider presentation helper.
    public var symbolName: String?
    public var fallbackInitials: String

    public init(
        providerId: String,
        title: String,
        fillFraction: Double,
        remainingText: String,
        detailText: String,
        level: GaryxUsageLevel,
        available: Bool,
        symbolName: String?,
        fallbackInitials: String
    ) {
        self.providerId = providerId
        self.title = title
        self.fillFraction = fillFraction
        self.remainingText = remainingText
        self.detailText = detailText
        self.level = level
        self.available = available
        self.symbolName = symbolName
        self.fallbackInitials = fallbackInitials
    }

    public static func make(from provider: GaryxProviderUsage, now: Date = Date()) -> GaryxUsageGaugeModel {
        let presentation = GaryxProviderPresentation.make(providerType: provider.id)
        guard provider.available, let weekly = provider.weekly else {
            let reason = (provider.error?.isEmpty == false) ? "Unavailable" : "No data"
            return GaryxUsageGaugeModel(
                providerId: provider.id,
                title: provider.name,
                fillFraction: 0,
                remainingText: "—",
                detailText: reason,
                level: .unavailable,
                available: false,
                symbolName: presentation.symbolName,
                fallbackInitials: presentation.fallbackInitials
            )
        }
        let remaining = weekly.remainingPercent.clamped(to: 0...100)
        let detail = resetDetailText(for: weekly, stale: provider.stale, now: now)
        return GaryxUsageGaugeModel(
            providerId: provider.id,
            title: provider.name,
            fillFraction: remaining / 100,
            remainingText: "\(Int(remaining.rounded()))%",
            detailText: detail,
            level: level(forRemaining: remaining),
            available: true,
            symbolName: presentation.symbolName,
            fallbackInitials: presentation.fallbackInitials
        )
    }

    public static func placeholder(
        providerId: String,
        title: String,
        remainingPercent: Double
    ) -> GaryxUsageGaugeModel {
        let presentation = GaryxProviderPresentation.make(providerType: providerId)
        let remaining = remainingPercent.clamped(to: 0...100)
        return GaryxUsageGaugeModel(
            providerId: providerId,
            title: title,
            fillFraction: remaining / 100,
            remainingText: "\(Int(remaining.rounded()))%",
            detailText: "weekly left",
            level: level(forRemaining: remaining),
            available: true,
            symbolName: presentation.symbolName,
            fallbackInitials: presentation.fallbackInitials
        )
    }

    static func level(forRemaining remaining: Double) -> GaryxUsageLevel {
        switch remaining {
        case let value where value >= 50:
            return .healthy
        case let value where value >= 20:
            return .warning
        default:
            return .critical
        }
    }

    static func resetDetailText(for window: GaryxUsageWindow, stale: Bool, now: Date) -> String {
        if stale {
            return "stale data"
        }
        if let seconds = resetSeconds(for: window, now: now), seconds > 0 {
            return "resets in \(formatDuration(seconds))"
        }
        return "weekly left"
    }

    static func resetSeconds(for window: GaryxUsageWindow, now: Date) -> Int? {
        if let resetsAt = window.resetsAt,
           let date = GaryxUsageDateParsing.date(fromISO8601: resetsAt) {
            return max(0, Int(date.timeIntervalSince(now)))
        }
        if let seconds = window.resetAfterSeconds {
            return max(0, seconds)
        }
        return nil
    }

    /// Compact human duration: `2d`, `5h`, `12m`, `<1m`.
    static func formatDuration(_ seconds: Int) -> String {
        let total = max(0, seconds)
        let days = total / 86_400
        if days >= 1 {
            return "\(days)d"
        }
        let hours = total / 3_600
        if hours >= 1 {
            return "\(hours)h"
        }
        let minutes = total / 60
        if minutes >= 1 {
            return "\(minutes)m"
        }
        return "<1m"
    }
}

public enum GaryxUsageDateParsing {
    private static let withFractional: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()

    private static let plain: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter
    }()

    public static func date(fromISO8601 value: String) -> Date? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        return withFractional.date(from: trimmed) ?? plain.date(from: trimmed)
    }
}

public enum GaryxCodingUsageWidgetConstants {
    public static let kind = "GaryxCodingUsageWidget"
    /// Provider ids exposed by the gateway, in display order.
    public static let claudeCodeProviderId = "claude_code"
    public static let codexProviderId = "codex"
}

extension Comparable {
    func clamped(to range: ClosedRange<Self>) -> Self {
        min(max(self, range.lowerBound), range.upperBound)
    }
}
