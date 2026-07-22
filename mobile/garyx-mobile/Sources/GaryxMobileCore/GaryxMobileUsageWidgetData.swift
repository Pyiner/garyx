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

/// A provider-defined usage window scoped to a named model or model family.
/// Claude Code currently uses this shape for Fable's independent weekly quota.
public struct GaryxScopedUsageLimit: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var name: String
    public var kind: String
    public var window: GaryxUsageWindow

    public init(id: String, name: String, kind: String, window: GaryxUsageWindow) {
        self.id = id
        self.name = name
        self.kind = kind
        self.window = window
    }
}

/// One per-model quota bucket as reported by coding providers that expose
/// model-scoped allowance, currently Antigravity.
public struct GaryxModelUsage: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var name: String
    public var remainingFraction: Double
    public var remainingPercent: Double
    public var usedPercent: Double
    public var resetsAt: String?
    public var resetAfterSeconds: Int?
    public var description: String?

    public init(
        id: String,
        name: String,
        remainingFraction: Double,
        remainingPercent: Double,
        usedPercent: Double,
        resetsAt: String? = nil,
        resetAfterSeconds: Int? = nil,
        description: String? = nil
    ) {
        self.id = id
        self.name = name
        self.remainingFraction = remainingFraction
        self.remainingPercent = remainingPercent
        self.usedPercent = usedPercent
        self.resetsAt = resetsAt
        self.resetAfterSeconds = resetAfterSeconds
        self.description = description
    }

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case remainingFraction = "remaining_fraction"
        case remainingPercent = "remaining_percent"
        case usedPercent = "used_percent"
        case resetsAt = "resets_at"
        case resetAfterSeconds = "reset_after_seconds"
        case description
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        remainingFraction = try container.decodeIfPresent(Double.self, forKey: .remainingFraction) ?? 0
        remainingPercent = try container.decodeIfPresent(Double.self, forKey: .remainingPercent)
            ?? (remainingFraction * 100)
        usedPercent = try container.decodeIfPresent(Double.self, forKey: .usedPercent)
            ?? max(0, 100 - remainingPercent)
        resetsAt = try container.decodeIfPresent(String.self, forKey: .resetsAt)
        resetAfterSeconds = try container.decodeIfPresent(Int.self, forKey: .resetAfterSeconds)
        description = try container.decodeIfPresent(String.self, forKey: .description)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(name, forKey: .name)
        try container.encode(remainingFraction, forKey: .remainingFraction)
        try container.encode(remainingPercent, forKey: .remainingPercent)
        try container.encode(usedPercent, forKey: .usedPercent)
        try container.encodeIfPresent(resetsAt, forKey: .resetsAt)
        try container.encodeIfPresent(resetAfterSeconds, forKey: .resetAfterSeconds)
        try container.encodeIfPresent(description, forKey: .description)
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
    public var scopedLimits: [GaryxScopedUsageLimit]
    public var models: [GaryxModelUsage]
    public var error: String?
    public var errorCode: String?
    public var retryAfterSeconds: Int?

    public init(
        id: String,
        name: String,
        available: Bool,
        stale: Bool = false,
        plan: String? = nil,
        weekly: GaryxUsageWindow? = nil,
        session: GaryxUsageWindow? = nil,
        scopedLimits: [GaryxScopedUsageLimit] = [],
        models: [GaryxModelUsage] = [],
        error: String? = nil,
        errorCode: String? = nil,
        retryAfterSeconds: Int? = nil
    ) {
        self.id = id
        self.name = name
        self.available = available
        self.stale = stale
        self.plan = plan
        self.weekly = weekly
        self.session = session
        self.scopedLimits = scopedLimits
        self.models = models
        self.error = error
        self.errorCode = errorCode
        self.retryAfterSeconds = retryAfterSeconds
    }

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case available
        case stale
        case plan
        case weekly
        case session
        case scopedLimits = "scoped_limits"
        case models
        case error
        case errorCode = "error_code"
        case retryAfterSeconds = "retry_after_seconds"
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
        scopedLimits = try container.decodeIfPresent([GaryxScopedUsageLimit].self, forKey: .scopedLimits) ?? []
        models = try container.decodeIfPresent([GaryxModelUsage].self, forKey: .models) ?? []
        error = try container.decodeIfPresent(String.self, forKey: .error)
        errorCode = try container.decodeIfPresent(String.self, forKey: .errorCode)
        retryAfterSeconds = try container.decodeIfPresent(Int.self, forKey: .retryAfterSeconds)
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
        if !scopedLimits.isEmpty {
            try container.encode(scopedLimits, forKey: .scopedLimits)
        }
        if !models.isEmpty {
            try container.encode(models, forKey: .models)
        }
        try container.encodeIfPresent(error, forKey: .error)
        try container.encodeIfPresent(errorCode, forKey: .errorCode)
        try container.encodeIfPresent(retryAfterSeconds, forKey: .retryAfterSeconds)
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
    /// Stale readings dim their gauge in the Quota hero (design §4); the
    /// widget conveys freshness through its snapshot "updated … ago" label
    /// instead and ignores this flag.
    public var stale: Bool

    public init(
        providerId: String,
        title: String,
        fillFraction: Double,
        remainingText: String,
        detailText: String,
        level: GaryxUsageLevel,
        available: Bool,
        symbolName: String?,
        fallbackInitials: String,
        stale: Bool = false
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
        self.stale = stale
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
                fallbackInitials: presentation.fallbackInitials,
                stale: provider.stale
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
            fallbackInitials: presentation.fallbackInitials,
            stale: provider.stale
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

    public static func widgetModels(
        from usage: GaryxCodingUsage,
        now: Date = Date()
    ) -> [GaryxUsageGaugeModel] {
        GaryxCodingUsageWidgetConstants.widgetProviderIds.compactMap { id in
            usage.provider(id: id).map { GaryxUsageGaugeModel.make(from: $0, now: now) }
        }
    }

    /// Gauge models for the provider-page Quota hero (design §6.4/D8): one
    /// gauge per metered provider in fixed order, keeping a "No data"
    /// placeholder for providers missing from the response so the hero holds
    /// stable columns as the widget deep-link landing.
    public static func heroModels(
        from usage: GaryxCodingUsage?,
        now: Date = Date()
    ) -> [GaryxUsageGaugeModel] {
        GaryxCodingUsageWidgetConstants.heroProviderIds.map { id in
            guard let provider = usage?.provider(id: id) else {
                return heroPlaceholder(providerId: id)
            }
            return heroModel(from: provider, now: now)
        }
    }

    /// Like `make`, but a provider that reports only per-model quota buckets
    /// (Antigravity) gauges its tightest bucket instead of reading "No data"
    /// (design §4.2 compact aggregation); weekly-window providers are
    /// unchanged.
    public static func heroModel(from provider: GaryxProviderUsage, now: Date = Date()) -> GaryxUsageGaugeModel {
        guard provider.available, provider.weekly == nil,
              let tightest = provider.models.min(by: { $0.remainingPercent < $1.remainingPercent }) else {
            return make(from: provider, now: now)
        }
        let presentation = GaryxProviderPresentation.make(providerType: provider.id)
        let remaining = tightest.remainingPercent.clamped(to: 0...100)
        let detail = provider.stale
            ? "stale data"
            : resetText(
                resetsAt: tightest.resetsAt,
                resetAfterSeconds: tightest.resetAfterSeconds,
                fallback: "tightest model",
                now: now
            )
        return GaryxUsageGaugeModel(
            providerId: provider.id,
            title: provider.name,
            fillFraction: remaining / 100,
            remainingText: "\(Int(remaining.rounded()))%",
            detailText: detail,
            level: level(forRemaining: remaining),
            available: true,
            symbolName: presentation.symbolName,
            fallbackInitials: presentation.fallbackInitials,
            stale: provider.stale
        )
    }

    private static func heroPlaceholder(providerId: String) -> GaryxUsageGaugeModel {
        let presentation = GaryxProviderPresentation.make(providerType: providerId)
        return GaryxUsageGaugeModel(
            providerId: providerId,
            title: presentation.displayName,
            fillFraction: 0,
            remainingText: "—",
            detailText: "No data",
            level: .unavailable,
            available: false,
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
        resetSeconds(resetsAt: window.resetsAt, resetAfterSeconds: window.resetAfterSeconds, now: now)
    }

    /// Cross-platform reset rule (design §4/D9): when both `reset_after_seconds`
    /// and `resets_at` are present and disagree, prefer the shorter conservative
    /// value — the same rule as the desktop `usageResetSeconds` helper.
    static func resetSeconds(resetsAt: String?, resetAfterSeconds: Int?, now: Date) -> Int? {
        var candidates: [Int] = []
        if let seconds = resetAfterSeconds {
            candidates.append(max(0, seconds))
        }
        if let resetsAt,
           let date = GaryxUsageDateParsing.date(fromISO8601: resetsAt) {
            candidates.append(max(0, Int(date.timeIntervalSince(now))))
        }
        return candidates.min()
    }

    /// `resets in 2d 4h` caption shared by every usage surface; falls back to
    /// the supplied text when no reset source is present.
    static func resetText(
        resetsAt: String?,
        resetAfterSeconds: Int?,
        fallback: String,
        now: Date
    ) -> String {
        if let seconds = resetSeconds(resetsAt: resetsAt, resetAfterSeconds: resetAfterSeconds, now: now) {
            return "resets in \(formatDuration(seconds))"
        }
        return fallback
    }

    /// `updated 3m ago` freshness label from the response `refreshed_at`, so a
    /// stale reading never looks freshly green (design §4).
    public static func usageUpdatedText(refreshedAt: String?, now: Date = Date()) -> String? {
        guard let refreshedAt,
              let date = GaryxUsageDateParsing.date(fromISO8601: refreshedAt) else {
            return nil
        }
        let seconds = max(0, Int(now.timeIntervalSince(date)))
        return "updated \(formatDuration(seconds)) ago"
    }

    /// Compact human duration matching the shared §4 spec and the desktop
    /// `formatUsageDuration`: `2d 4h`, `1h 12m`, `12m`, `<1m`.
    static func formatDuration(_ seconds: Int) -> String {
        let total = max(0, seconds)
        let days = total / 86_400
        let hours = (total % 86_400) / 3_600
        let minutes = (total % 3_600) / 60
        if days >= 1 {
            return hours > 0 ? "\(days)d \(hours)h" : "\(days)d"
        }
        if hours >= 1 {
            return minutes > 0 ? "\(hours)h \(minutes)m" : "\(hours)h"
        }
        if minutes >= 1 {
            return "\(minutes)m"
        }
        return "<1m"
    }
}

public struct GaryxProviderModelUsageDisplayModel: Equatable, Identifiable, Sendable {
    public var id: String
    public var title: String
    /// Remaining fill for the mini-bar, clamped 0-100.
    public var remainingPercent: Double
    public var remainingText: String
    public var detailText: String
    public var level: GaryxUsageLevel

    public init(
        id: String,
        title: String,
        remainingPercent: Double = 0,
        remainingText: String,
        detailText: String,
        level: GaryxUsageLevel
    ) {
        self.id = id
        self.title = title
        self.remainingPercent = remainingPercent
        self.remainingText = remainingText
        self.detailText = detailText
        self.level = level
    }
}

/// One labelled remaining-quota meter (`Session` 5h / `Weekly` 7d) per the
/// shared §4 visualization spec.
public struct GaryxProviderUsageWindowDisplayModel: Equatable, Identifiable, Sendable {
    public var id: String { label }
    public var label: String
    /// Remaining fill for the meter, clamped 0-100.
    public var remainingPercent: Double
    public var remainingText: String
    public var detailText: String
    public var level: GaryxUsageLevel

    public init(
        label: String,
        remainingPercent: Double,
        remainingText: String,
        detailText: String,
        level: GaryxUsageLevel
    ) {
        self.label = label
        self.remainingPercent = remainingPercent
        self.remainingText = remainingText
        self.detailText = detailText
        self.level = level
    }
}

public struct GaryxProviderUsageDisplayModel: Equatable, Sendable {
    public var providerId: String
    public var summaryText: String
    public var detailText: String
    public var available: Bool
    /// Subscription plan name (`Max`, `Pro`) shown as a pill by default (D4).
    public var plan: String?
    /// Stale readings dim their meters and surface `updatedText` (design §4).
    public var stale: Bool
    /// `updated 3m ago`, derived from the response `refreshed_at` when known.
    public var updatedText: String?
    /// Session, weekly, and provider-scoped meters in display order.
    public var windows: [GaryxProviderUsageWindowDisplayModel]
    /// Per-model quota buckets (Antigravity), tightest remaining first.
    public var models: [GaryxProviderModelUsageDisplayModel]

    public init(
        providerId: String,
        summaryText: String,
        detailText: String,
        available: Bool,
        plan: String? = nil,
        stale: Bool = false,
        updatedText: String? = nil,
        windows: [GaryxProviderUsageWindowDisplayModel] = [],
        models: [GaryxProviderModelUsageDisplayModel]
    ) {
        self.providerId = providerId
        self.summaryText = summaryText
        self.detailText = detailText
        self.available = available
        self.plan = plan
        self.stale = stale
        self.updatedText = updatedText
        self.windows = windows
        self.models = models
    }

    public static func make(
        from provider: GaryxProviderUsage?,
        refreshedAt: String? = nil,
        now: Date = Date()
    ) -> GaryxProviderUsageDisplayModel? {
        guard let provider else { return nil }
        let plan = provider.plan?.trimmingCharacters(in: .whitespacesAndNewlines)
        let normalizedPlan = (plan?.isEmpty == false) ? plan : nil
        let updatedText = GaryxUsageGaugeModel.usageUpdatedText(refreshedAt: refreshedAt, now: now)
        if !provider.available {
            let unavailableCopy = unavailableCopy(for: provider)
            return GaryxProviderUsageDisplayModel(
                providerId: provider.id,
                summaryText: unavailableCopy.summary,
                detailText: unavailableCopy.detail,
                available: false,
                plan: normalizedPlan,
                stale: provider.stale,
                updatedText: updatedText,
                models: []
            )
        }

        let modelRows = provider.models
            .map { model -> GaryxProviderModelUsageDisplayModel in
                let remaining = model.remainingPercent.clamped(to: 0...100)
                return GaryxProviderModelUsageDisplayModel(
                    id: model.id,
                    title: model.name,
                    remainingPercent: remaining,
                    remainingText: "\(Int(remaining.rounded()))% left",
                    detailText: modelDetailText(for: model, stale: provider.stale, now: now),
                    level: GaryxUsageGaugeModel.level(forRemaining: remaining)
                )
            }
            .sorted { $0.remainingPercent < $1.remainingPercent }

        var windows: [GaryxProviderUsageWindowDisplayModel] = []
        if let session = provider.session {
            windows.append(windowDisplayModel(label: "Session", window: session, fallback: "session window", now: now))
        }
        if let weekly = provider.weekly {
            windows.append(windowDisplayModel(label: "Weekly", window: weekly, fallback: "weekly window", now: now))
        }
        for limit in provider.scopedLimits {
            windows.append(
                windowDisplayModel(
                    label: limit.name,
                    window: limit.window,
                    fallback: limit.kind.contains("weekly") ? "weekly window" : "usage window",
                    now: now
                )
            )
        }

        if let weekly = provider.weekly {
            let remaining = weekly.remainingPercent.clamped(to: 0...100)
            return GaryxProviderUsageDisplayModel(
                providerId: provider.id,
                summaryText: "\(Int(remaining.rounded()))% left",
                detailText: GaryxUsageGaugeModel.resetDetailText(for: weekly, stale: provider.stale, now: now),
                available: true,
                plan: normalizedPlan,
                stale: provider.stale,
                updatedText: updatedText,
                windows: windows,
                models: modelRows
            )
        }

        if !windows.isEmpty {
            let tightest = windows.min { $0.remainingPercent < $1.remainingPercent }
            return GaryxProviderUsageDisplayModel(
                providerId: provider.id,
                summaryText: tightest.map { "\(Int($0.remainingPercent.rounded()))% left" } ?? "No data",
                detailText: tightest?.detailText ?? "Usage not reported",
                available: true,
                plan: normalizedPlan,
                stale: provider.stale,
                updatedText: updatedText,
                windows: windows,
                models: modelRows
            )
        }

        if !modelRows.isEmpty {
            let modelCountText = modelRows.count == 1
                ? "1 model quota"
                : "\(modelRows.count) model quotas"
            return GaryxProviderUsageDisplayModel(
                providerId: provider.id,
                summaryText: modelCountText,
                detailText: provider.stale ? "stale data" : "Per-model quota",
                available: true,
                plan: normalizedPlan,
                stale: provider.stale,
                updatedText: updatedText,
                models: modelRows
            )
        }

        return GaryxProviderUsageDisplayModel(
            providerId: provider.id,
            summaryText: "No data",
            detailText: provider.error?.isEmpty == false ? unavailableCopy(for: provider).detail : "Usage not reported",
            available: false,
            plan: normalizedPlan,
            stale: provider.stale,
            updatedText: updatedText,
            models: []
        )
    }

    private static func unavailableCopy(
        for provider: GaryxProviderUsage
    ) -> (summary: String, detail: String) {
        switch provider.errorCode {
        case "rate_limited":
            if let seconds = provider.retryAfterSeconds, seconds > 0 {
                return (
                    "Try again in \(GaryxUsageGaugeModel.formatDuration(seconds))",
                    "Claude quota is temporarily rate limited"
                )
            }
            return ("Temporarily limited", "Claude quota is temporarily rate limited")
        case "reauth_required":
            return ("Sign in again", "Claude Code credentials expired")
        case "credentials_unavailable":
            return ("Account unavailable", "Claude Code credentials were not found")
        case "network":
            return ("Can’t refresh quota", "Check your connection and try again")
        default:
            return provider.error?.isEmpty == false
                ? ("Quota unavailable", "Try again shortly")
                : ("No data", "No usage data")
        }
    }

    private static func windowDisplayModel(
        label: String,
        window: GaryxUsageWindow,
        fallback: String,
        now: Date
    ) -> GaryxProviderUsageWindowDisplayModel {
        let remaining = window.remainingPercent.clamped(to: 0...100)
        return GaryxProviderUsageWindowDisplayModel(
            label: label,
            remainingPercent: remaining,
            remainingText: "\(Int(remaining.rounded()))%",
            detailText: GaryxUsageGaugeModel.resetText(
                resetsAt: window.resetsAt,
                resetAfterSeconds: window.resetAfterSeconds,
                fallback: fallback,
                now: now
            ),
            level: GaryxUsageGaugeModel.level(forRemaining: remaining)
        )
    }

    private static func modelDetailText(
        for model: GaryxModelUsage,
        stale: Bool,
        now: Date
    ) -> String {
        if stale {
            return "stale data"
        }
        let description = model.description?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !description.isEmpty {
            return description
        }
        let window = GaryxUsageWindow(
            usedPercent: model.usedPercent,
            remainingPercent: model.remainingPercent,
            resetsAt: model.resetsAt,
            resetAfterSeconds: model.resetAfterSeconds
        )
        return GaryxUsageGaugeModel.resetDetailText(for: window, stale: false, now: now)
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
    public static let antigravityProviderId = "antigravity"
    /// The widget stays Claude + Codex only (design D7).
    public static let widgetProviderIds = [claudeCodeProviderId, codexProviderId]
    /// The in-app Quota hero shows all three metered providers (design D8).
    public static let heroProviderIds = [claudeCodeProviderId, codexProviderId, antigravityProviderId]
}

/// Deep link for the whole "Garyx Quota" widget (design §8/D7): opens the
/// provider settings page, whose top is the Quota hero. The widget target
/// compiles this file but not the full `GaryxMobileRouteLink` builder, so —
/// like `GaryxMobileThreadLink` — this mirrors the canonical route URL; a Core
/// test pins it to `GaryxMobileRouteLink.make(.settings(.provider))`.
public enum GaryxMobileProviderSettingsLink {
    public static func make() -> URL? {
        var components = URLComponents()
        components.scheme = "garyx"
        components.host = "mobile"
        components.path = "/settings/provider"
        return components.url
    }
}

extension Comparable {
    func clamped(to range: ClosedRange<Self>) -> Self {
        min(max(self, range.lowerBound), range.upperBound)
    }
}
