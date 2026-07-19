import SwiftUI
import WidgetKit

struct GaryxCodingUsageEntry: TimelineEntry {
    let date: Date
    let snapshot: GaryxUsageWidgetSnapshot
    let isPlaceholder: Bool

    init(date: Date, snapshot: GaryxUsageWidgetSnapshot, isPlaceholder: Bool = false) {
        self.date = date
        self.snapshot = snapshot
        self.isPlaceholder = isPlaceholder
    }
}

struct GaryxCodingUsageProvider: TimelineProvider {
    /// Requested cadence. iOS budgets widget refreshes, so the system may
    /// coalesce these; one minute is the floor we ask for.
    private static let refreshInterval: TimeInterval = 60

    func placeholder(in context: Context) -> GaryxCodingUsageEntry {
        GaryxCodingUsageEntry(date: Date(), snapshot: Self.sampleSnapshot, isPlaceholder: true)
    }

    func getSnapshot(in context: Context, completion: @escaping (GaryxCodingUsageEntry) -> Void) {
        // Use sample data only for the widget gallery preview; a real placed
        // widget must reflect the actual (possibly empty) app-warmed snapshot.
        if context.isPreview {
            completion(GaryxCodingUsageEntry(date: Date(), snapshot: Self.sampleSnapshot, isPlaceholder: true))
            return
        }
        completion(GaryxCodingUsageEntry(date: Date(), snapshot: GaryxUsageWidgetStore.loadSnapshot()))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<GaryxCodingUsageEntry>) -> Void) {
        // The widget renders the app-warmed snapshot from the shared App Group;
        // the app owns the authenticated gateway fetch. An empty snapshot renders
        // the "No usage yet" state rather than sample data. We re-render on the
        // requested cadence so the "updated … ago" label stays current.
        let now = Date()
        let entry = GaryxCodingUsageEntry(date: now, snapshot: GaryxUsageWidgetStore.loadSnapshot())
        let next = now.addingTimeInterval(Self.refreshInterval)
        completion(Timeline(entries: [entry], policy: .after(next)))
    }

    static let sampleSnapshot = GaryxUsageWidgetSnapshot(
        usage: GaryxCodingUsage(
            providers: [
                GaryxProviderUsage(
                    id: GaryxCodingUsageWidgetConstants.claudeCodeProviderId,
                    name: "Claude Code",
                    available: true,
                    plan: "max",
                    weekly: GaryxUsageWindow(usedPercent: 27, remainingPercent: 73, resetAfterSeconds: 5 * 86_400)
                ),
                GaryxProviderUsage(
                    id: GaryxCodingUsageWidgetConstants.codexProviderId,
                    name: "Codex",
                    available: true,
                    plan: "pro",
                    weekly: GaryxUsageWindow(usedPercent: 89, remainingPercent: 11, resetAfterSeconds: 2 * 86_400)
                ),
            ]
        ),
        fetchedAt: Date()
    )
}

// MARK: - Views
//
// The speedometer itself (`GaryxUsageSpeedometer` + metrics) lives in
// `GaryxUsageGaugeView.swift`, shared with the app's provider-page Quota hero.

struct GaryxCodingUsageWidgetView: View {
    let entry: GaryxCodingUsageEntry

    @Environment(\.widgetFamily) private var widgetFamily

    private var metrics: GaryxCodingUsageMetrics {
        GaryxCodingUsageMetrics(family: widgetFamily)
    }

    private var models: [GaryxUsageGaugeModel] {
        GaryxUsageGaugeModel.widgetModels(from: entry.snapshot.usage, now: entry.date)
    }

    private var ageText: String? {
        guard !entry.isPlaceholder else { return nil }
        return entry.snapshot.ageText(asOf: entry.date)
    }

    var body: some View {
        linkedContent
            // WidgetKit families are fixed, non-scrolling canvases; the Core
            // boundary records the XXL cap and why the gauges stay reachable.
            .garyxWidgetTypographyBoundary(.widgetFamilyChrome)
            .containerBackground(for: .widget) {
                ContainerRelativeShape().fill(.thinMaterial)
            }
    }

    // One tap target, one destination (the provider settings page, whose top
    // is the Quota hero), for content and empty state alike. Medium/large
    // express it as an explicit whole-card Link element (widget rule: no
    // container-level widgetURL where Link semantics exist). systemSmall is
    // the platform exception: WidgetKit ignores Link controls there, so the
    // small family keeps the equivalent family-scoped widgetURL — the only
    // supported tap mechanism, with no per-element links it could steal.
    @ViewBuilder
    private var linkedContent: some View {
        if widgetFamily == .systemSmall {
            paddedContent.widgetURL(GaryxMobileProviderSettingsLink.make())
        } else if let url = GaryxMobileProviderSettingsLink.make() {
            Link(destination: url) { paddedContent }
                .buttonStyle(.plain)
        } else {
            paddedContent
        }
    }

    private var paddedContent: some View {
        Group {
            if models.isEmpty {
                emptyState
            } else {
                VStack(spacing: 8) {
                    if metrics.showsHeader {
                        HStack {
                            Text("Weekly quota left")
                                .garyxRelativePointFont(
                                    size: metrics.headerSize,
                                    relativeTo: .subheadline,
                                    weight: .semibold
                                )
                                .foregroundStyle(.secondary)
                            Spacer()
                            if let ageText {
                                Text(ageText)
                                    .garyxRelativePointFont(
                                        size: metrics.detailSize,
                                        relativeTo: .caption,
                                        weight: .medium
                                    )
                                    .foregroundStyle(.tertiary)
                            }
                        }
                    }
                    HStack(spacing: metrics.gaugeSpacing) {
                        ForEach(models, id: \.providerId) { model in
                            GaryxUsageSpeedometer(model: model, metrics: metrics)
                        }
                    }
                    .frame(maxHeight: .infinity)
                    if !metrics.showsHeader, metrics.showsDetail, let ageText {
                        Text(ageText)
                            .garyxRelativePointFont(
                                size: metrics.detailSize - 1,
                                relativeTo: .caption2,
                                weight: .medium
                            )
                            .foregroundStyle(.tertiary)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(metrics.contentPadding)
    }

    private var emptyState: some View {
        VStack(spacing: 4) {
            Image(systemName: "gauge.with.dots.needle.50percent")
                .font(.system(size: 22, weight: .semibold))
                .foregroundStyle(.secondary)
            Text("No usage yet")
                .garyxRelativePointFont(size: 14, relativeTo: .body, weight: .semibold)
                .foregroundStyle(.primary)
            Text("Open Garyx to connect")
                .garyxRelativePointFont(size: 11.5, relativeTo: .caption, weight: .medium)
                .foregroundStyle(.secondary)
        }
        .multilineTextAlignment(.center)
    }
}

struct GaryxCodingUsageWidget: Widget {
    let kind = GaryxCodingUsageWidgetConstants.kind

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: GaryxCodingUsageProvider()) { entry in
            GaryxCodingUsageWidgetView(entry: entry)
        }
        .configurationDisplayName("Garyx Quota")
        .description("Weekly quota left for Claude Code and Codex.")
        .supportedFamilies([.systemSmall, .systemMedium, .systemLarge])
        .contentMarginsDisabled()
    }
}
