import SwiftUI
import WidgetKit

// The speedometer gauge shared by the "Garyx Quota" widget and the in-app
// provider-page Quota hero (design §6.4/D8). This file is compiled into BOTH
// the app target and the widget extension (see project.yml), so the two
// surfaces render one gauge implementation, driven by the Core-public
// `GaryxUsageGaugeModel`. Keep it free of app-only chrome (`GaryxTheme`,
// `GaryxFont`): the widget's rendering is the visual contract.

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

struct GaryxUsageSpeedometer: View {
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
        // "quota", not "weekly quota": the hero's Antigravity gauge reads its
        // tightest per-model bucket; the detail text carries the window.
        return "\(model.title): \(model.remainingText) of quota left, \(model.detailText)"
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
