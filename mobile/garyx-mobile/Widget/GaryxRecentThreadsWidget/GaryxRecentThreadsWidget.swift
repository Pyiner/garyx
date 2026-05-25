import SwiftUI
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
                    GaryxMobileWidgetThread(id: "thread::sample-1", title: "Mobile release polish", workspaceName: "Garyx"),
                    GaryxMobileWidgetThread(id: "thread::sample-2", title: "Automation follow-up", workspaceName: "Garyx"),
                    GaryxMobileWidgetThread(id: "thread::sample-3", title: "Gateway runtime check", workspaceName: "Garyx"),
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
        Array(entry.snapshot.threads.prefix(GaryxMobileWidgetStore.threadLimit))
    }

    private var metrics: GaryxRecentThreadsWidgetMetrics {
        GaryxRecentThreadsWidgetMetrics(family: widgetFamily)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: metrics.headerToListSpacing) {
            HStack(spacing: 6) {
                Image(systemName: "bubble.left.and.text.bubble.right.fill")
                    .font(.system(size: metrics.headerIconSize, weight: .semibold))
                    .foregroundStyle(.primary)
                Text("Gary X")
                    .font(.system(size: metrics.headerFontSize, weight: .semibold))
                    .foregroundStyle(.primary)
                Spacer(minLength: 6)
                Text("Recent")
                    .font(.system(size: metrics.headerBadgeFontSize, weight: .medium))
                    .foregroundStyle(.secondary)
            }

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
                VStack(spacing: metrics.rowSpacing) {
                    ForEach(threads) { thread in
                        if let url = GaryxMobileThreadLink.make(threadId: thread.id) {
                            Link(destination: url) {
                                GaryxRecentThreadWidgetRow(thread: thread, metrics: metrics)
                            }
                        } else {
                            GaryxRecentThreadWidgetRow(thread: thread, metrics: metrics)
                        }
                    }
                }
            }
        }
        .padding(metrics.contentPadding)
        .containerBackground(.background, for: .widget)
        .widgetURL(threads.first.flatMap { GaryxMobileThreadLink.make(threadId: $0.id) })
    }
}

private struct GaryxRecentThreadsWidgetMetrics {
    let contentPadding: CGFloat
    let headerToListSpacing: CGFloat
    let headerIconSize: CGFloat
    let headerFontSize: CGFloat
    let headerBadgeFontSize: CGFloat
    let rowSpacing: CGFloat
    let rowMinHeight: CGFloat
    let rowDotSize: CGFloat
    let rowTitleFontSize: CGFloat
    let rowWorkspaceFontSize: CGFloat
    let rowWorkspaceSpacing: CGFloat
    let rowShowsWorkspace: Bool

    init(family: WidgetFamily) {
        switch family {
        case .systemMedium:
            contentPadding = 12
            headerToListSpacing = 7
            headerIconSize = 13
            headerFontSize = 13
            headerBadgeFontSize = 10
            rowSpacing = 2
            rowMinHeight = 19
            rowDotSize = 5
            rowTitleFontSize = 11
            rowWorkspaceFontSize = 9
            rowWorkspaceSpacing = 0
            rowShowsWorkspace = false
        default:
            contentPadding = 14
            headerToListSpacing = 8
            headerIconSize = 14
            headerFontSize = 14
            headerBadgeFontSize = 11
            rowSpacing = 5
            rowMinHeight = 22
            rowDotSize = 6
            rowTitleFontSize = 12
            rowWorkspaceFontSize = 10
            rowWorkspaceSpacing = 1
            rowShowsWorkspace = true
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
        HStack(spacing: 7) {
            Circle()
                .fill(isRunning ? Color.accentColor : Color.secondary.opacity(0.4))
                .frame(width: metrics.rowDotSize, height: metrics.rowDotSize)

            VStack(alignment: .leading, spacing: metrics.rowWorkspaceSpacing) {
                Text(thread.title)
                    .font(.system(size: metrics.rowTitleFontSize, weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)

                if metrics.rowShowsWorkspace, !thread.workspaceName.isEmpty {
                    Text(thread.workspaceName)
                        .font(.system(size: metrics.rowWorkspaceFontSize, weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }

            Spacer(minLength: 4)

            Image(systemName: "chevron.right")
                .font(.system(size: 9, weight: .semibold))
                .foregroundStyle(.tertiary)
        }
        .frame(maxWidth: .infinity, minHeight: metrics.rowMinHeight, alignment: .leading)
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
