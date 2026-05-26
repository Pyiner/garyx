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

    private var metrics: GaryxRecentThreadsWidgetMetrics {
        GaryxRecentThreadsWidgetMetrics(family: widgetFamily)
    }

    private var threads: [GaryxMobileWidgetThread] {
        Array(entry.snapshot.threads.prefix(metrics.visibleRowCount))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if threads.isEmpty {
                Spacer(minLength: 0)
                Text("No recent threads")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(.primary)
                Text("Open Gary X to refresh")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.secondary)
                Spacer(minLength: 0)
            } else {
                VStack(spacing: metrics.rowSpacing) {
                    ForEach(threads) { thread in
                        if let url = GaryxMobileThreadLink.make(threadId: thread.id) {
                            Link(destination: url) {
                                GaryxRecentThreadWidgetRow(thread: thread, metrics: metrics)
                            }
                            .buttonStyle(.plain)
                        } else {
                            GaryxRecentThreadWidgetRow(thread: thread, metrics: metrics)
                        }
                    }
                    Spacer(minLength: 0)
                }
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
    let visibleRowCount: Int

    init(family: WidgetFamily) {
        switch family {
        case .systemMedium:
            contentPadding = 14
            rowContentSpacing = 11
            rowSpacing = 4
            rowHeight = 38
            avatarSize = 28
            avatarIconSize = 12
            runningDotSize = 6
            rowTitleFontSize = 13
            rowWorkspaceFontSize = 10.5
            rowWorkspaceSpacing = 1
            visibleRowCount = 3
        case .systemExtraLarge:
            contentPadding = 18
            rowContentSpacing = 13
            rowSpacing = 6
            rowHeight = 46
            avatarSize = 34
            avatarIconSize = 14
            runningDotSize = 7
            rowTitleFontSize = 14.5
            rowWorkspaceFontSize = 11.5
            rowWorkspaceSpacing = 2
            visibleRowCount = 6
        default:
            contentPadding = 16
            rowContentSpacing = 12
            rowSpacing = 6
            rowHeight = 44
            avatarSize = 32
            avatarIconSize = 13
            runningDotSize = 7
            rowTitleFontSize = 14
            rowWorkspaceFontSize = 11
            rowWorkspaceSpacing = 2
            visibleRowCount = 6
        }
    }
}

private struct GaryxRecentThreadWidgetRow: View {
    let thread: GaryxMobileWidgetThread
    let metrics: GaryxRecentThreadsWidgetMetrics

    private var isRunning: Bool {
        let activeRun = thread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let runState = thread.runState?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        return !activeRun.isEmpty || runState == "running"
    }

    var body: some View {
        HStack(spacing: metrics.rowContentSpacing) {
            GaryxWidgetAgentAvatar(thread: thread, metrics: metrics)

            VStack(alignment: .leading, spacing: metrics.rowWorkspaceSpacing) {
                HStack(spacing: 6) {
                    Text(thread.title)
                        .font(.system(size: metrics.rowTitleFontSize, weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                        .truncationMode(.middle)

                    if isRunning {
                        GaryxWidgetRunningIndicator(size: metrics.runningDotSize)
                    }
                }

                if !thread.workspaceName.isEmpty {
                    Text(thread.workspaceName)
                        .font(.system(size: metrics.rowWorkspaceFontSize, weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }

            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, minHeight: metrics.rowHeight, maxHeight: metrics.rowHeight, alignment: .leading)
        .contentShape(Rectangle())
    }
}

private struct GaryxWidgetRunningIndicator: View {
    let size: CGFloat

    var body: some View {
        ZStack {
            Circle()
                .stroke(Color.primary.opacity(0.14), lineWidth: 1)
            Circle()
                .trim(from: 0.12, to: 0.78)
                .stroke(
                    Color.primary.opacity(0.58),
                    style: StrokeStyle(lineWidth: 1.35, lineCap: .round)
                )
                .rotationEffect(.degrees(34))
        }
        .frame(width: max(size + 4, 8), height: max(size + 4, 8))
        .accessibilityLabel("Running")
    }
}

private struct GaryxWidgetAgentAvatar: View {
    let thread: GaryxMobileWidgetThread
    let metrics: GaryxRecentThreadsWidgetMetrics

    var body: some View {
        ZStack {
            Circle()
                .fill(Color.primary.opacity(0.08))

            if let image = GaryxWidgetDataURLImageCache.image(from: thread.avatarDataUrl) {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
                    .frame(width: metrics.avatarSize, height: metrics.avatarSize)
                    .clipShape(Circle())
            } else if thread.isTeam {
                Image(systemName: "person.2.fill")
                    .font(.system(size: metrics.avatarIconSize, weight: .semibold))
                    .foregroundStyle(.secondary)
            } else if let symbol = providerPresentation.symbolName {
                Image(systemName: symbol)
                    .font(.system(size: metrics.avatarIconSize, weight: .semibold))
                    .foregroundStyle(.secondary)
            } else {
                Text(providerPresentation.fallbackInitials)
                    .font(.system(size: metrics.avatarIconSize, weight: .bold))
                    .foregroundStyle(.secondary)
                    .minimumScaleFactor(0.7)
            }
        }
        .frame(width: metrics.avatarSize, height: metrics.avatarSize)
        .accessibilityHidden(true)
    }

    private var providerPresentation: GaryxProviderPresentation {
        GaryxProviderPresentation.make(
            agentId: thread.agentId,
            providerType: thread.providerType,
            fallbackName: thread.agentName ?? thread.title
        )
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
