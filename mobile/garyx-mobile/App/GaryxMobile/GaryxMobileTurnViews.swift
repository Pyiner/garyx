import SwiftUI

/// Named coordinate space attached to the transcript CONTENT stack (not the
/// scroll viewport). Positions measured in it are scroll-invariant: they only
/// change when the layout itself changes, which is what makes it the right
/// ruler for older-history prepend compensation — the anchor row's
/// content-space displacement IS the exact height inserted above it,
/// unaffected by concurrent tail growth or reader scrolling.
let garyxConversationContentSpaceName = "garyx-conversation-content"

struct GaryxMobileTurnRowsView: View {
    @Environment(\.garyxMotion) private var motion
    let rows: [GaryxMobileTurnRow]
    let prefetchBoundaryRowCount: Int
    let onNearHistoryBoundary: () -> Void
    let onRowContentMinYChange: (_ rowId: String, _ minY: CGFloat) -> Void

    init(
        rows: [GaryxMobileTurnRow],
        prefetchBoundaryRowCount: Int = 0,
        onNearHistoryBoundary: @escaping () -> Void = {},
        onRowContentMinYChange: @escaping (_ rowId: String, _ minY: CGFloat) -> Void = { _, _ in }
    ) {
        self.rows = rows
        self.prefetchBoundaryRowCount = prefetchBoundaryRowCount
        self.onNearHistoryBoundary = onNearHistoryBoundary
        self.onRowContentMinYChange = onRowContentMinYChange
    }

    var body: some View {
        ForEach(Array(rows.enumerated()), id: \.element.id) { rowIndex, row in
            // The row wrapper VStack exists so the whole turn row has ONE
            // geometry to observe. Its spacing matches the transcript stack,
            // so the wrapped layout stays pixel-identical to the previously
            // flattened children.
            VStack(alignment: .leading, spacing: 14) {
                turnRowContent(rowIndex: rowIndex, row: row)
            }
            .onGeometryChange(for: CGFloat.self) { proxy in
                proxy.frame(in: .named(garyxConversationContentSpaceName)).minY
            } action: { minY in
                onRowContentMinYChange(row.id, minY)
            }
            .onAppear {
                guard rowIndex <= prefetchBoundaryRowCount else { return }
                onNearHistoryBoundary()
            }
        }
    }

    @ViewBuilder
    private func turnRowContent(rowIndex: Int, row: GaryxMobileTurnRow) -> some View {
        if let userBlock = row.userBlock {
            GaryxMobileTranscriptBlockView(block: userBlock)
                .transition(motion.transition(.transcriptAppear))
        }

        ForEach(Array(row.activityRows.enumerated()), id: \.element.id) { _, activityRow in
            GaryxMobileTurnActivityRowView(row: activityRow)
                .transition(motion.transition(.transcriptAppear))
        }

        // Server render_state appends capsule cards after the turn's final
        // answer. Dumb-render only — placement and existence are server-derived.
        if !row.capsuleCards.isEmpty {
            GaryxMobileCapsuleChatCardsView(
                turnId: row.id,
                cards: row.capsuleCards
            )
            .transition(motion.transition(.transcriptAppear))
        }
    }
}

struct GaryxMobileTurnActivityRowView: View {
    let row: GaryxMobileTurnRow.ActivityRow

    var body: some View {
        switch row {
        case .flat(let block):
            GaryxMobileTranscriptBlockView(block: block)
        case .turn(let turn):
            GaryxTurnSummaryView(turn: turn) {
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
    @Environment(\.garyxMotion) private var motion
    let turn: GaryxMobileAgentTurn
    let content: Content

    @State private var expanded: Bool
    @State private var userControlled = false
    @State private var mountStart = Date()

    init(
        turn: GaryxMobileAgentTurn,
        @ViewBuilder content: () -> Content
    ) {
        self.turn = turn
        self.content = content()
        _expanded = State(initialValue: turn.isRunning)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                userControlled = true
                withAnimation(motion.animation(.turnDisclosure)) {
                    expanded.toggle()
                }
            } label: {
                HStack(spacing: 8) {
                    summaryText
                        .fixedSize(horizontal: true, vertical: false)

                    Rectangle()
                        .fill(GaryxTheme.hairline)
                        .frame(height: 1)

                    Image(systemName: "chevron.down")
                        .font(GaryxFont.system(size: 10, weight: .semibold))
                        .foregroundStyle(GaryxTheme.secondaryText)
                        .rotationEffect(.degrees(expanded ? 0 : -90))
                        .animation(motion.spatialAnimation(.turnDisclosure), value: expanded)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(expanded ? "Collapse turn details" : "Expand turn details")

            if expanded && turn.hasBody {
                VStack(alignment: .leading, spacing: 14) {
                    content
                }
                .transition(motion.transition(.turnDisclosure, moveFrom: .top))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onChange(of: isRunning) { _, isRunning in
            guard !userControlled else { return }
            withAnimation(motion.animation(.turnAutoDisclosure)) {
                expanded = isRunning
            }
        }
    }

    private var isRunning: Bool {
        turn.isRunning
    }

    @ViewBuilder
    private var summaryText: some View {
        if isRunning {
            TimelineView(.periodic(from: Date(), by: 1)) { context in
                GaryxShimmerText(
                    text: summaryLabel(now: context.date),
                    font: GaryxFont.footnote(weight: .medium)
                )
                .lineLimit(1)
            }
        } else {
            summaryTextLabel(summaryLabel(now: Date()))
        }
    }

    private func summaryTextLabel(_ label: String) -> some View {
        Text(label)
            .font(GaryxFont.footnote(weight: .medium))
            .foregroundStyle(isRunning ? GaryxTheme.accent : GaryxTheme.secondaryText)
            .lineLimit(1)
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
