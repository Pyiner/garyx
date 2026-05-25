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

    private var threads: [GaryxMobileWidgetThread] {
        Array(entry.snapshot.threads.prefix(GaryxMobileWidgetStore.threadLimit))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: "bubble.left.and.text.bubble.right.fill")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(.primary)
                Text("Gary X")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(.primary)
                Spacer(minLength: 6)
                Text("Recent")
                    .font(.system(size: 11, weight: .medium))
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
                VStack(spacing: 5) {
                    ForEach(threads) { thread in
                        if let url = GaryxMobileThreadLink.make(threadId: thread.id) {
                            Link(destination: url) {
                                GaryxRecentThreadWidgetRow(thread: thread)
                            }
                        } else {
                            GaryxRecentThreadWidgetRow(thread: thread)
                        }
                    }
                }
            }
        }
        .padding(14)
        .containerBackground(.background, for: .widget)
        .widgetURL(threads.first.flatMap { GaryxMobileThreadLink.make(threadId: $0.id) })
    }
}

private struct GaryxRecentThreadWidgetRow: View {
    let thread: GaryxMobileWidgetThread

    private var isRunning: Bool {
        let activeRun = thread.activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let runState = thread.runState?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        return !activeRun.isEmpty || runState == "running"
    }

    var body: some View {
        HStack(spacing: 7) {
            Circle()
                .fill(isRunning ? Color.accentColor : Color.secondary.opacity(0.4))
                .frame(width: 6, height: 6)

            VStack(alignment: .leading, spacing: 1) {
                Text(thread.title)
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)

                if !thread.workspaceName.isEmpty {
                    Text(thread.workspaceName)
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }

            Spacer(minLength: 4)

            Image(systemName: "chevron.right")
                .font(.system(size: 9, weight: .semibold))
                .foregroundStyle(.tertiary)
        }
        .frame(maxWidth: .infinity, minHeight: 22, alignment: .leading)
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
