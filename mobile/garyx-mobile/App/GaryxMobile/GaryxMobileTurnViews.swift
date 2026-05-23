import SwiftUI

struct GaryxMobileTurnRowsView: View {
    let rows: [GaryxMobileTurnRow]
    let forceRunningLastTurn: Bool

    var body: some View {
        ForEach(Array(rows.enumerated()), id: \.element.id) { rowIndex, row in
            if let userBlock = row.userBlock {
                GaryxMobileTranscriptBlockView(block: userBlock)
            }

            ForEach(Array(row.activityRows.enumerated()), id: \.element.id) { activityIndex, activityRow in
                let forceRunning = forceRunningLastTurn
                    && rowIndex == rows.count - 1
                    && activityIndex == row.activityRows.count - 1
                GaryxMobileTurnActivityRowView(
                    row: activityRow,
                    forceRunning: forceRunning
                )
            }
        }
    }
}

struct GaryxMobileTurnActivityRowView: View {
    let row: GaryxMobileTurnRow.ActivityRow
    let forceRunning: Bool

    var body: some View {
        switch row {
        case .flat(let block):
            GaryxMobileTranscriptBlockView(block: block)
        case .turn(let turn):
            GaryxTurnSummaryView(
                turn: turn,
                forceRunning: forceRunning && turn.finalBlock == nil
            ) {
                ForEach(turn.steps) { step in
                    GaryxMobileTranscriptBlockView(block: step)
                }
            }
            if let finalBlock = turn.finalBlock {
                GaryxMobileTranscriptBlockView(block: finalBlock)
            }
        }
    }
}

struct GaryxMobileTranscriptBlockView: View {
    let block: GaryxMobileTranscriptBlock

    var body: some View {
        switch block {
        case .message(let message), .toolGroup(let message):
            GaryxMessageBubble(message: message)
                .id(message.id)
        }
    }
}

struct GaryxTurnSummaryView<Content: View>: View {
    let turn: GaryxMobileAgentTurn
    let forceRunning: Bool
    let content: Content

    @State private var expanded: Bool
    @State private var userControlled = false
    @State private var mountStart = Date()

    init(
        turn: GaryxMobileAgentTurn,
        forceRunning: Bool,
        @ViewBuilder content: () -> Content
    ) {
        self.turn = turn
        self.forceRunning = forceRunning
        self.content = content()
        _expanded = State(initialValue: turn.isRunning || forceRunning)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                userControlled = true
                withAnimation(.easeOut(duration: 0.18)) {
                    expanded.toggle()
                }
            } label: {
                HStack(spacing: 8) {
                    TimelineView(.periodic(from: Date(), by: 1)) { context in
                        let label = summaryLabel(now: context.date)
                        if isRunning {
                            GaryxShimmerText(
                                text: label,
                                font: GaryxFont.footnote(weight: .medium)
                            )
                            .lineLimit(1)
                        } else {
                            Text(label)
                                .font(GaryxFont.footnote(weight: .medium))
                                .foregroundStyle(GaryxTheme.secondaryText)
                                .lineLimit(1)
                        }
                    }
                    .fixedSize(horizontal: true, vertical: false)

                    Rectangle()
                        .fill(GaryxTheme.hairline)
                        .frame(height: 1)

                    Image(systemName: "chevron.down")
                        .font(GaryxFont.system(size: 10, weight: .semibold))
                        .foregroundStyle(GaryxTheme.secondaryText)
                        .rotationEffect(.degrees(expanded ? 0 : -90))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(expanded ? "Collapse turn details" : "Expand turn details")

            if expanded && turn.hasBody {
                VStack(alignment: .leading, spacing: 14) {
                    content
                }
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onChange(of: isRunning) { _, isRunning in
            guard !userControlled else { return }
            withAnimation(.easeOut(duration: 0.2)) {
                expanded = isRunning
            }
        }
    }

    private var isRunning: Bool {
        turn.isRunning || forceRunning
    }

    private func summaryLabel(now: Date) -> String {
        let elapsed = elapsedLabel(now: now)
        if isRunning {
            return elapsed.isEmpty ? "Working" : "Working for \(elapsed)"
        }
        return elapsed.isEmpty ? "Worked" : "Worked for \(elapsed)"
    }

    private func elapsedLabel(now: Date) -> String {
        let start = Self.timestamp(from: turn.startedAt)
        if isRunning {
            let start = start ?? mountStart
            return Self.formatElapsed(now.timeIntervalSince(start))
        }
        guard let start, let finished = Self.timestamp(from: turn.finishedAt) else {
            return ""
        }
        return Self.formatElapsed(finished.timeIntervalSince(start))
    }

    private static func timestamp(from value: String?) -> Date? {
        guard let value else { return nil }
        return ISO8601DateFormatter.garyxMobileFractional.date(from: value)
            ?? ISO8601DateFormatter.garyxMobileInternet.date(from: value)
    }

    private static func formatElapsed(_ seconds: TimeInterval) -> String {
        let safe = max(0, Int(seconds.rounded()))
        if safe < 60 {
            return "\(safe)s"
        }
        let minutes = safe / 60
        let remainder = safe % 60
        if minutes < 60 {
            return remainder > 0 ? "\(minutes)m \(remainder)s" : "\(minutes)m"
        }
        let hours = minutes / 60
        let restMinutes = minutes % 60
        return restMinutes > 0 ? "\(hours)h \(restMinutes)m" : "\(hours)h"
    }
}

private extension ISO8601DateFormatter {
    static let garyxMobileFractional: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()

    static let garyxMobileInternet: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter
    }()
}
