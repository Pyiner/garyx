import SwiftUI
import UIKit
import WidgetKit

struct GaryxRecentThreadsEntry: TimelineEntry {
    let date: Date
    let snapshot: GaryxMobileWidgetSnapshot
}

struct GaryxRecentThreadsProvider: TimelineProvider {
    func placeholder(in context: Context) -> GaryxRecentThreadsEntry {
        GaryxRecentThreadsEntry(
            date: Date(),
            snapshot: GaryxMobileWidgetSnapshot(
                threads: [
                    GaryxMobileWidgetThread(
                        id: "thread::sample-1",
                        title: "Design review",
                        workspaceName: "Local workspace",
                        agentId: "codex-agent",
                        agentName: "Codex Agent",
                        providerType: "codex",
                        builtIn: true
                    ),
                    GaryxMobileWidgetThread(
                        id: "thread::sample-2",
                        title: "Mobile polish",
                        workspaceName: "Local workspace",
                        activeRunId: "run::sample",
                        runState: "running",
                        agentId: "claude-agent",
                        agentName: "Claude Agent",
                        providerType: "claude",
                        builtIn: true
                    ),
                    GaryxMobileWidgetThread(
                        id: "thread::sample-3",
                        title: "Agent handoff",
                        workspaceName: "Demo bot",
                        agentId: "demo-team",
                        agentName: "Demo Team",
                        isTeam: true
                    ),
                    GaryxMobileWidgetThread(id: "thread::sample-4", title: "Release notes", workspaceName: "Garyx"),
                    GaryxMobileWidgetThread(id: "thread::sample-5", title: "Gateway runtime check", workspaceName: "Garyx"),
                    GaryxMobileWidgetThread(id: "thread::sample-6", title: "Automation follow-up", workspaceName: "Garyx"),
                ]
            )
        )
    }

    func getSnapshot(in context: Context, completion: @escaping (GaryxRecentThreadsEntry) -> Void) {
        completion(GaryxRecentThreadsEntry(date: Date(), snapshot: GaryxMobileWidgetStore.loadRecentThreads()))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<GaryxRecentThreadsEntry>) -> Void) {
        let now = Date()
        let entry = GaryxRecentThreadsEntry(date: now, snapshot: GaryxMobileWidgetStore.loadRecentThreads())
        let nextRefresh = Calendar.current.date(byAdding: .minute, value: 10, to: now) ?? now.addingTimeInterval(600)
        completion(Timeline(entries: [entry], policy: .after(nextRefresh)))
    }
}

struct GaryxRecentThreadsWidgetView: View {
    let entry: GaryxRecentThreadsEntry

    @Environment(\.widgetFamily) private var widgetFamily

    private var threads: [GaryxMobileWidgetThread] {
        entry.snapshot.threads
    }

    private var metrics: GaryxRecentThreadsWidgetMetrics {
        GaryxRecentThreadsWidgetMetrics(family: widgetFamily)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if threads.isEmpty {
                Spacer(minLength: 0)
                Text("No recent threads")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.primary)
                Text("Open Gary X to refresh")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
                Spacer(minLength: 0)
            } else {
                ScrollView(.vertical) {
                    LazyVStack(spacing: metrics.rowSpacing) {
                        ForEach(Array(threads.enumerated()), id: \.element.id) { index, thread in
                            if let url = GaryxMobileThreadLink.make(threadId: thread.id) {
                                Link(destination: url) {
                                    GaryxRecentThreadWidgetRow(
                                        thread: thread,
                                        metrics: metrics,
                                        showsSeparator: index < threads.count - 1
                                    )
                                }
                                .buttonStyle(.plain)
                            } else {
                                GaryxRecentThreadWidgetRow(
                                    thread: thread,
                                    metrics: metrics,
                                    showsSeparator: index < threads.count - 1
                                )
                            }
                        }
                    }
                }
                .scrollIndicators(.hidden)
                .frame(maxHeight: metrics.visibleListHeight)
            }
        }
        .padding(metrics.contentPadding)
        .containerBackground(for: .widget) {
            ContainerRelativeShape()
                .fill(.thinMaterial)
        }
        .widgetURL(threads.first.flatMap { GaryxMobileThreadLink.make(threadId: $0.id) })
    }
}

private struct GaryxRecentThreadsWidgetMetrics {
    let contentPadding: CGFloat
    let rowContentSpacing: CGFloat
    let rowSpacing: CGFloat
    let rowHeight: CGFloat
    let avatarSize: CGFloat
    let avatarIconSize: CGFloat
    let runningDotSize: CGFloat
    let rowTitleFontSize: CGFloat
    let rowWorkspaceFontSize: CGFloat
    let rowWorkspaceSpacing: CGFloat
    let chevronSize: CGFloat

    var visibleListHeight: CGFloat {
        let visibleRows = CGFloat(GaryxMobileWidgetStore.visibleThreadLimit)
        let visibleSpacings = max(0, visibleRows - 1) * rowSpacing
        return rowHeight * visibleRows + visibleSpacings
    }

    init(family: WidgetFamily) {
        switch family {
        case .systemMedium:
            contentPadding = 12
            rowContentSpacing = 8
            rowSpacing = 0
            rowHeight = 24
            avatarSize = 19
            avatarIconSize = 8
            runningDotSize = 5
            rowTitleFontSize = 10.5
            rowWorkspaceFontSize = 8.5
            rowWorkspaceSpacing = 0
            chevronSize = 8
        default:
            contentPadding = 14
            rowContentSpacing = 10
            rowSpacing = 0
            rowHeight = 32
            avatarSize = 24
            avatarIconSize = 10
            runningDotSize = 6
            rowTitleFontSize = 12
            rowWorkspaceFontSize = 9.5
            rowWorkspaceSpacing = 1
            chevronSize = 9
        }
    }
}

private struct GaryxRecentThreadWidgetRow: View {
    let thread: GaryxMobileWidgetThread
    let metrics: GaryxRecentThreadsWidgetMetrics
    let showsSeparator: Bool

    private var isRunning: Bool {
        let activeRun = thread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let runState = thread.runState?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        return !activeRun.isEmpty || runState == "running"
    }

    var body: some View {
        HStack(spacing: metrics.rowContentSpacing) {
            GaryxWidgetAgentAvatar(thread: thread, metrics: metrics)

            VStack(alignment: .leading, spacing: metrics.rowWorkspaceSpacing) {
                Text(thread.title)
                    .font(.system(size: metrics.rowTitleFontSize, weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)

                if !thread.workspaceName.isEmpty {
                    Text(thread.workspaceName)
                        .font(.system(size: metrics.rowWorkspaceFontSize, weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }

            Spacer(minLength: 4)

            if isRunning {
                Circle()
                    .fill(Color.accentColor)
                    .frame(width: metrics.runningDotSize, height: metrics.runningDotSize)
            }

            Image(systemName: "chevron.right")
                .font(.system(size: metrics.chevronSize, weight: .semibold))
                .foregroundStyle(.tertiary)
        }
        .frame(maxWidth: .infinity, minHeight: metrics.rowHeight, maxHeight: metrics.rowHeight, alignment: .leading)
        .contentShape(Rectangle())
        .overlay(alignment: .bottom) {
            if showsSeparator {
                Rectangle()
                    .fill(Color.primary.opacity(0.08))
                    .frame(height: 0.5)
                    .padding(.leading, metrics.avatarSize + metrics.rowContentSpacing)
            }
        }
    }
}

private struct GaryxWidgetAgentAvatar: View {
    let thread: GaryxMobileWidgetThread
    let metrics: GaryxRecentThreadsWidgetMetrics

    var body: some View {
        ZStack {
            Circle()
                .fill(fallbackBackground)

            if let image = GaryxWidgetDataURLImageCache.image(from: thread.avatarDataUrl) {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
                    .frame(width: metrics.avatarSize, height: metrics.avatarSize)
                    .clipShape(Circle())
            } else if thread.isTeam {
                Image(systemName: "person.2.fill")
                    .font(.system(size: metrics.avatarIconSize, weight: .semibold))
                    .foregroundStyle(fallbackForeground)
            } else if let symbol = providerSymbol {
                Image(systemName: symbol)
                    .font(.system(size: metrics.avatarIconSize, weight: .semibold))
                    .foregroundStyle(fallbackForeground)
            } else {
                Text(initials)
                    .font(.system(size: metrics.avatarIconSize, weight: .bold))
                    .foregroundStyle(fallbackForeground)
                    .minimumScaleFactor(0.7)
            }
        }
        .frame(width: metrics.avatarSize, height: metrics.avatarSize)
        .overlay {
            Circle()
                .stroke(Color.primary.opacity(0.07), lineWidth: 1)
        }
        .accessibilityHidden(true)
    }

    private var providerSymbol: String? {
        let source = "\(thread.agentId ?? "") \(thread.providerType ?? "")".lowercased()
        if source.contains("codex") || source.contains("openai") || source.contains("gpt") {
            return "sparkles"
        }
        if source.contains("claude") || source.contains("anthropic") {
            return "brain.head.profile"
        }
        if source.contains("gemini") || source.contains("google") {
            return "diamond.fill"
        }
        return nil
    }

    private var initials: String {
        let source = (thread.agentName ?? thread.agentId ?? thread.title)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !source.isEmpty else { return "A" }
        let words = source
            .replacingOccurrences(of: "(", with: " ")
            .replacingOccurrences(of: ")", with: " ")
            .split { $0 == " " || $0 == "/" || $0 == "_" || $0 == "-" }
        if words.count >= 2, let first = words[0].first, let second = words[1].first {
            return "\(first)\(second)".uppercased()
        }
        return String(source.prefix(2)).uppercased()
    }

    private var fallbackBackground: Color {
        if thread.builtIn {
            return Color.primary.opacity(0.10)
        }
        let colors = [
            Color(red: 0.86, green: 0.93, blue: 0.96),
            Color(red: 0.91, green: 0.88, blue: 0.97),
            Color(red: 0.92, green: 0.96, blue: 0.88),
            Color(red: 0.97, green: 0.91, blue: 0.86),
        ]
        let source = "\(thread.agentName ?? "")\(thread.agentId ?? "")\(thread.title)"
        let seed = source.unicodeScalars.reduce(0) { ($0 &+ Int($1.value)) % 997 }
        return colors[seed % colors.count]
    }

    private var fallbackForeground: Color {
        thread.builtIn ? Color.primary.opacity(0.72) : Color.primary.opacity(0.66)
    }
}

private enum GaryxWidgetDataURLImageCache {
    private static let cache: NSCache<NSString, UIImage> = {
        let cache = NSCache<NSString, UIImage>()
        cache.countLimit = 64
        cache.totalCostLimit = 16 * 1024 * 1024
        return cache
    }()

    static func image(from rawValue: String?) -> UIImage? {
        let raw = (rawValue ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty else { return nil }
        let cacheKey = NSString(string: raw)
        if let cached = cache.object(forKey: cacheKey) {
            return cached
        }
        let encoded = raw.split(separator: ",", maxSplits: 1).last.map(String.init) ?? raw
        guard let data = Data(base64Encoded: encoded),
              let image = UIImage(data: data) else {
            return nil
        }
        cache.setObject(image, forKey: cacheKey, cost: data.count)
        return image
    }
}

struct GaryxRecentThreadsWidget: Widget {
    let kind = GaryxRecentThreadsWidgetConstants.kind

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: GaryxRecentThreadsProvider()) { entry in
            GaryxRecentThreadsWidgetView(entry: entry)
        }
        .configurationDisplayName("Gary X Recent")
        .description("Open recent Gary X threads.")
        .supportedFamilies([.systemMedium, .systemLarge, .systemExtraLarge])
    }
}

@main
struct GaryxRecentThreadsWidgetBundle: WidgetBundle {
    var body: some Widget {
        GaryxRecentThreadsWidget()
    }
}
