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

private extension GaryxUsageLevel {
    var tint: Color {
        switch self {
        case .healthy: return Color(red: 0.18, green: 0.78, blue: 0.45)
        case .warning: return Color(red: 0.98, green: 0.66, blue: 0.16)
        case .critical: return Color(red: 0.95, green: 0.30, blue: 0.30)
        case .unavailable: return Color.secondary
        }
    }
}

/// A 270° speedometer arc. `fillFraction` is 0...1 of the sweep. The radius is
/// inset by half the stroke width so a thick ring never clips at the edges.
private struct GaryxUsageGaugeArc: Shape {
    var fillFraction: Double
    var lineWidth: CGFloat

    func path(in rect: CGRect) -> Path {
        let radius = max(0, (min(rect.width, rect.height) - lineWidth) / 2)
        let center = CGPoint(x: rect.midX, y: rect.midY)
        let sweep = 270.0 * fillFraction.clamped(to: 0...1)
        var path = Path()
        path.addArc(
            center: center,
            radius: radius,
            startAngle: .degrees(135),
            endAngle: .degrees(135 + sweep),
            clockwise: false
        )
        return path
    }
}

private struct GaryxUsageSpeedometer: View {
    let model: GaryxUsageGaugeModel
    var metrics: GaryxCodingUsageMetrics

    var body: some View {
        VStack(spacing: metrics.gaugeLabelSpacing) {
            ZStack {
                GaryxUsageGaugeArc(fillFraction: 1, lineWidth: metrics.gaugeLineWidth)
                    .stroke(
                        Color.primary.opacity(0.10),
                        style: StrokeStyle(lineWidth: metrics.gaugeLineWidth, lineCap: .round)
                    )
                GaryxUsageGaugeArc(
                    fillFraction: model.available ? model.fillFraction : 0,
                    lineWidth: metrics.gaugeLineWidth
                )
                .stroke(
                    model.level.tint,
                    style: StrokeStyle(lineWidth: metrics.gaugeLineWidth, lineCap: .round)
                )

                VStack(spacing: 1) {
                    if let symbol = model.symbolName {
                        Image(systemName: symbol)
                            .font(.system(size: metrics.gaugeIconSize, weight: .semibold))
                            .foregroundStyle(.secondary)
                    }
                    Text(model.remainingText)
                        .font(.system(size: metrics.gaugeValueSize, weight: .bold, design: .rounded))
                        .foregroundStyle(model.available ? .primary : .secondary)
                        .minimumScaleFactor(0.6)
                        .lineLimit(1)
                }
                .padding(.horizontal, 2)
            }
            .aspectRatio(1, contentMode: .fit)
            .frame(maxWidth: metrics.gaugeMaxWidth)

            VStack(spacing: 0) {
                Text(model.title)
                    .font(.system(size: metrics.titleSize, weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
                if metrics.showsDetail {
                    Text(model.detailText)
                        .font(.system(size: metrics.detailSize, weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                }
            }
        }
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityLabel)
    }

    private var accessibilityLabel: String {
        guard model.available else { return "\(model.title): usage unavailable" }
        return "\(model.title): \(model.remainingText) of weekly quota left, \(model.detailText)"
    }
}

struct GaryxCodingUsageMetrics {
    var contentPadding: CGFloat
    var gaugeSpacing: CGFloat
    var gaugeLineWidth: CGFloat
    var gaugeValueSize: CGFloat
    var gaugeIconSize: CGFloat
    var gaugeLabelSpacing: CGFloat
    var gaugeMaxWidth: CGFloat?
    var titleSize: CGFloat
    var detailSize: CGFloat
    var headerSize: CGFloat
    var showsDetail: Bool
    var showsHeader: Bool

    init(family: WidgetFamily) {
        switch family {
        case .systemSmall:
            contentPadding = 12
            gaugeSpacing = 8
            gaugeLineWidth = 7
            gaugeValueSize = 17
            gaugeIconSize = 9
            gaugeLabelSpacing = 3
            gaugeMaxWidth = 62
            titleSize = 10.5
            detailSize = 9
            headerSize = 11
            showsDetail = false
            showsHeader = false
        case .systemLarge, .systemExtraLarge:
            contentPadding = 22
            gaugeSpacing = 24
            gaugeLineWidth = 14
            gaugeValueSize = 40
            gaugeIconSize = 18
            gaugeLabelSpacing = 8
            gaugeMaxWidth = 200
            titleSize = 18
            detailSize = 14
            headerSize = 15
            showsDetail = true
            showsHeader = true
        default: // systemMedium
            contentPadding = 16
            gaugeSpacing = 18
            gaugeLineWidth = 10
            gaugeValueSize = 26
            gaugeIconSize = 13
            gaugeLabelSpacing = 5
            gaugeMaxWidth = 104
            titleSize = 13
            detailSize = 11
            headerSize = 12
            showsDetail = true
            showsHeader = false
        }
    }
}

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
        Group {
            if models.isEmpty {
                emptyState
            } else {
                VStack(spacing: 8) {
                    if metrics.showsHeader {
                        HStack {
                            Text("Weekly quota left")
                                .font(.system(size: metrics.headerSize, weight: .semibold))
                                .foregroundStyle(.secondary)
                            Spacer()
                            if let ageText {
                                Text(ageText)
                                    .font(.system(size: metrics.detailSize, weight: .medium))
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
                            .font(.system(size: metrics.detailSize - 1, weight: .medium))
                            .foregroundStyle(.tertiary)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(metrics.contentPadding)
        .containerBackground(for: .widget) {
            ContainerRelativeShape().fill(.thinMaterial)
        }
    }

    private var emptyState: some View {
        VStack(spacing: 4) {
            Image(systemName: "gauge.with.dots.needle.50percent")
                .font(.system(size: 22, weight: .semibold))
                .foregroundStyle(.secondary)
            Text("No usage yet")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(.primary)
            Text("Open Garyx to connect")
                .font(.system(size: 11.5, weight: .medium))
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
