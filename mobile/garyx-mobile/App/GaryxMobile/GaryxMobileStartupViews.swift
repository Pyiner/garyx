import SwiftUI

/// Branded startup loading screen shown while saved gateway settings connect
/// directly, mirroring the Mac app's startup shell: centered Garyx mark,
/// "Starting Garyx" title, secondary status line, and a thin indeterminate
/// sliding progress bar on the warm page background.
struct GaryxStartupLoadingView: View {
    var body: some View {
        VStack(spacing: 0) {
            Spacer()

            Image("GaryxAppMark")
                .resizable()
                .scaledToFit()
                .frame(width: 116, height: 116)
                .shadow(color: Color(red: 0.10, green: 0.11, blue: 0.12).opacity(0.16), radius: 14, x: 0, y: 10)

            Text("Starting Garyx")
                .font(GaryxFont.body(weight: .semibold))
                .foregroundStyle(.primary)
                .padding(.top, 22)

            Text("Syncing workspace state and gateway status...")
                .font(GaryxFont.footnote())
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.top, 6)
                .padding(.horizontal, 32)

            GaryxStartupProgressBar()
                .padding(.top, 20)

            Spacer()
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .garyxPageBackground()
    }
}

/// Indeterminate sliding progress bar matching the Mac startup shell: a 42%
/// fill segment sweeping across a 172pt hairline track every 1.05s.
private struct GaryxStartupProgressBar: View {
    private let trackWidth: CGFloat = 172
    private let trackHeight: CGFloat = 3
    private let cycle: Double = 1.05

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 60.0)) { context in
            let raw = (context.date.timeIntervalSinceReferenceDate / cycle)
                .truncatingRemainder(dividingBy: 1.0)
            let eased = easeInOut(raw)
            let segmentWidth = trackWidth * 0.42
            let startX = -1.2 * segmentWidth
            let endX = 2.6 * segmentWidth
            let offset = startX + (endX - startX) * CGFloat(eased)

            Capsule()
                .fill(Color.primary.opacity(0.08))
                .frame(width: trackWidth, height: trackHeight)
                .overlay(alignment: .leading) {
                    Capsule()
                        .fill(Color.primary.opacity(0.6))
                        .frame(width: segmentWidth, height: trackHeight)
                        .offset(x: offset)
                }
                .clipShape(Capsule())
        }
        .frame(width: trackWidth, height: trackHeight)
        .accessibilityLabel("Connecting")
    }

    private func easeInOut(_ t: Double) -> Double {
        t < 0.5 ? 2 * t * t : 1 - pow(-2 * t + 2, 2) / 2
    }
}
