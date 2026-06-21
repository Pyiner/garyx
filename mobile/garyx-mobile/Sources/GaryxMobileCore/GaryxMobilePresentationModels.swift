import Foundation

enum GaryxThreadSummaryRunStateResolver {
    static func resolvedRunState(
        apiRunState: String?,
        recentRunId: String?,
        committedState: GaryxTranscriptRunState?
    ) -> String? {
        guard let committedState else {
            return apiRunState
        }

        if committedState.busy {
            return "running"
        }

        if let terminal = trimmed(committedState.terminalStatus) {
            return terminal
        }

        return hasValue(recentRunId) ? "completed" : "idle"
    }

    private static func trimmed(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }

    private static func hasValue(_ value: String?) -> Bool {
        !(value?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true)
    }
}

struct GaryxSidebarThreadRowPresentation: Equatable, Sendable {
    let title: String
    let subtitle: String?
    let trailingTimestamp: String?
    let isSelected: Bool
    let isPinned: Bool
    let isRunning: Bool

    init(
        thread: GaryxThreadSummary,
        isSelected: Bool,
        isPinned: Bool,
        trailingTimestamp: String?,
        showsRunningState: Bool = true
    ) {
        self.title = thread.title.isEmpty ? "Untitled" : thread.title
        self.subtitle = Self.subtitle(for: thread)
        self.trailingTimestamp = trailingTimestamp
        self.isSelected = isSelected
        self.isPinned = isPinned
        self.isRunning = showsRunningState && Self.isRunning(thread)
    }

    init(
        title: String,
        subtitle: String?,
        trailingTimestamp: String?,
        isSelected: Bool,
        isPinned: Bool,
        isRunning: Bool
    ) {
        self.title = title
        self.subtitle = subtitle
        self.trailingTimestamp = trailingTimestamp
        self.isSelected = isSelected
        self.isPinned = isPinned
        self.isRunning = isRunning
    }

    func withTrailingTimestamp(_ trailingTimestamp: String?) -> GaryxSidebarThreadRowPresentation {
        GaryxSidebarThreadRowPresentation(
            title: title,
            subtitle: subtitle,
            trailingTimestamp: trailingTimestamp,
            isSelected: isSelected,
            isPinned: isPinned,
            isRunning: isRunning
        )
    }

    func withRunningState(_ isRunning: Bool) -> GaryxSidebarThreadRowPresentation {
        GaryxSidebarThreadRowPresentation(
            title: title,
            subtitle: subtitle,
            trailingTimestamp: trailingTimestamp,
            isSelected: isSelected,
            isPinned: isPinned,
            isRunning: isRunning
        )
    }

    private static func subtitle(for thread: GaryxThreadSummary) -> String? {
        let context: String? = {
            if let workspacePath = thread.workspacePath, !workspacePath.isEmpty {
                return workspacePath.garyxLastPathComponent
            }
            if let teamName = thread.teamName, !teamName.isEmpty {
                return teamName
            }
            return thread.agentId
        }()
        let parts = [context, compactedPreview(thread.lastMessagePreview)].compactMap { part -> String? in
            let trimmed = part?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return trimmed.isEmpty ? nil : trimmed
        }
        return parts.isEmpty ? nil : parts.joined(separator: " \u{00B7} ")
    }

    /// Last-message previews can span lines; collapse to one display line.
    private static func compactedPreview(_ raw: String) -> String? {
        let collapsed = raw
            .components(separatedBy: .whitespacesAndNewlines)
            .filter { !$0.isEmpty }
            .joined(separator: " ")
        return collapsed.isEmpty ? nil : collapsed
    }

    private static func isRunning(_ thread: GaryxThreadSummary) -> Bool {
        let state = thread.runState?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return state == "running"
    }
}

struct GaryxAutomationDraft: Equatable {
    var label = ""
    var prompt = ""
    var agentTargetId = ""
    var schedule = GaryxAutomationScheduleDraft()
    var targetsExistingThread = false
    var targetThreadId = ""
    var workspacePath = ""

    var trimmedLabel: String {
        label.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var trimmedPrompt: String {
        prompt.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var trimmedWorkspacePath: String {
        workspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var trimmedAgentTargetId: String {
        agentTargetId.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var trimmedTargetThreadId: String {
        targetThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func canSubmit(workspacePaths: [String], threadOptions: [GaryxThreadSummary]) -> Bool {
        guard !trimmedLabel.isEmpty,
              !trimmedPrompt.isEmpty else {
            return false
        }
        if targetsExistingThread {
            return !effectiveThreadId(threadOptions: threadOptions).isEmpty
        }
        return !trimmedAgentTargetId.isEmpty && !effectiveWorkspacePath(workspacePaths: workspacePaths).isEmpty
    }

    func effectiveWorkspacePath(workspacePaths: [String]) -> String {
        if !trimmedWorkspacePath.isEmpty {
            return trimmedWorkspacePath
        }
        return workspacePaths.first ?? ""
    }

    func effectiveThreadId(threadOptions: [GaryxThreadSummary]) -> String {
        if !trimmedTargetThreadId.isEmpty {
            return trimmedTargetThreadId
        }
        return threadOptions.first?.id ?? ""
    }

    mutating func ensureTargetSelection(workspacePaths: [String], threadOptions: [GaryxThreadSummary]) {
        if targetsExistingThread {
            let nextThreadId = effectiveThreadId(threadOptions: threadOptions)
            targetThreadId = nextThreadId
            if let workspacePath = threadOptions.first(where: { $0.id == nextThreadId })?.workspacePath?
                .trimmingCharacters(in: .whitespacesAndNewlines),
               !workspacePath.isEmpty {
                self.workspacePath = workspacePath
            }
        } else {
            workspacePath = effectiveWorkspacePath(workspacePaths: workspacePaths)
        }
    }
}

enum GaryxAutomationRepeatOption: String, CaseIterable, Identifiable {
    case daily
    case weekdays
    case weekly
    case monthly
    case once
    case interval

    var id: String { rawValue }

    var label: String {
        switch self {
        case .daily:
            "Every Day"
        case .weekdays:
            "Weekdays"
        case .weekly:
            "Every Week"
        case .monthly:
            "Every Month"
        case .once:
            "No Repeat"
        case .interval:
            "Every N Hours"
        }
    }
}

struct GaryxAutomationScheduleDraft: Equatable {
    var repeatOption: GaryxAutomationRepeatOption = .daily
    var date: Date = Date()
    var time: Date = Self.defaultTime()
    var weekday: Int = Calendar.current.component(.weekday, from: Date())
    var monthDay: Int = Calendar.current.component(.day, from: Date())
    var intervalHours: Int = 24
    var timezone: String = TimeZone.current.identifier

    init() {}

    init(schedule: GaryxAutomationSchedule) {
        timezone = schedule.timezone?.trimmingCharacters(in: .whitespacesAndNewlines)
            .nonEmpty ?? TimeZone.current.identifier
        switch schedule.kind {
        case .daily:
            time = Self.dateForTime(schedule.time ?? "08:00")
            let normalized = schedule.weekdays.map { $0.lowercased() }
            if normalized == ["mo", "tu", "we", "th", "fr"] {
                repeatOption = .weekdays
                weekday = 2
            } else if normalized.count == 1 {
                repeatOption = .weekly
                weekday = Self.weekdayNumber(for: normalized[0])
            } else {
                repeatOption = .daily
            }
        case .interval:
            repeatOption = .interval
            intervalHours = max(schedule.hours ?? 24, 1)
        case .monthly:
            repeatOption = .monthly
            time = Self.dateForTime(schedule.time ?? "08:00")
            monthDay = min(max(schedule.day ?? Self.currentMonthDay, 1), 31)
        case .once:
            repeatOption = .once
            if let parsed = Self.dateForOnce(schedule.at) {
                date = parsed
                time = parsed
            }
        }
    }

    var schedule: GaryxAutomationSchedule {
        switch repeatOption {
        case .daily:
            return .daily(time: timeString, weekdays: [], timezone: timezone)
        case .weekdays:
            return .daily(time: timeString, weekdays: ["mo", "tu", "we", "th", "fr"], timezone: timezone)
        case .weekly:
            return .daily(time: timeString, weekdays: [Self.weekdayCode(for: weekday)], timezone: timezone)
        case .monthly:
            return .monthly(day: monthDay, time: timeString, timezone: timezone)
        case .once:
            return .once(at: onceString)
        case .interval:
            return .interval(hours: intervalHours)
        }
    }

    var timeString: String {
        let components = Calendar.current.dateComponents([.hour, .minute], from: time)
        return String(format: "%02d:%02d", components.hour ?? 8, components.minute ?? 0)
    }

    var onceString: String {
        let calendar = Calendar.current
        let dateComponents = calendar.dateComponents([.year, .month, .day], from: date)
        let timeComponents = calendar.dateComponents([.hour, .minute], from: time)
        var merged = DateComponents()
        merged.year = dateComponents.year
        merged.month = dateComponents.month
        merged.day = dateComponents.day
        merged.hour = timeComponents.hour
        merged.minute = timeComponents.minute
        let value = calendar.date(from: merged) ?? date
        return Self.onceFormatter.string(from: value)
    }

    private static var currentMonthDay: Int {
        Calendar.current.component(.day, from: Date())
    }

    private static func defaultTime() -> Date {
        dateForTime("08:00")
    }

    private static func dateForTime(_ value: String) -> Date {
        let parts = value.split(separator: ":")
        let hour = parts.first.flatMap { Int($0) } ?? 8
        let minute = parts.dropFirst().first.flatMap { Int($0) } ?? 0
        return Calendar.current.date(
            bySettingHour: min(max(hour, 0), 23),
            minute: min(max(minute, 0), 59),
            second: 0,
            of: Date()
        ) ?? Date()
    }

    private static func dateForOnce(_ value: String?) -> Date? {
        guard let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines),
              !trimmed.isEmpty else {
            return nil
        }
        return onceFormatter.date(from: trimmed)
            ?? isoFormatter.date(from: trimmed)
            ?? isoFormatterWithFractionalSeconds.date(from: trimmed)
    }

    private static let onceFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "yyyy-MM-dd'T'HH:mm"
        return formatter
    }()

    private static let isoFormatter: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter
    }()

    private static let isoFormatterWithFractionalSeconds: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()

    private static func weekdayCode(for weekday: Int) -> String {
        switch weekday {
        case 1: "su"
        case 2: "mo"
        case 3: "tu"
        case 4: "we"
        case 5: "th"
        case 6: "fr"
        case 7: "sa"
        default: "mo"
        }
    }

    private static func weekdayNumber(for code: String) -> Int {
        switch code {
        case "su", "sun", "sunday": 1
        case "mo", "mon", "monday": 2
        case "tu", "tue", "tuesday": 3
        case "we", "wed", "wednesday": 4
        case "th", "thu", "thursday": 5
        case "fr", "fri", "friday": 6
        case "sa", "sat", "saturday": 7
        default: Calendar.current.component(.weekday, from: Date())
        }
    }
}

private extension String {
    var nonEmpty: String? {
        isEmpty ? nil : self
    }
}
