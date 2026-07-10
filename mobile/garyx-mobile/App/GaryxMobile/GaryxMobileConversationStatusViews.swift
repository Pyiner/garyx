import Foundation
import SwiftUI

/// Transcript loading placeholder: a chat-shaped skeleton (user pill on the
/// trailing edge, assistant text lines on the leading edge) swept by the same
/// soft shimmer treatment as `GaryxShimmerText`, instead of a bare spinner.
struct GaryxThreadHistoryLoadingView: View {
    private static let shimmerDuration: Double = 2.4

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: false)) { context in
            let normalized = context.date.timeIntervalSinceReferenceDate
                .truncatingRemainder(dividingBy: Self.shimmerDuration) / Self.shimmerDuration
            let phase = CGFloat(normalized) * 2.0 - 0.5
            let fill = LinearGradient(
                colors: [
                    Color.primary.opacity(0.05),
                    Color.primary.opacity(0.11),
                    Color.primary.opacity(0.05),
                ],
                startPoint: UnitPoint(x: phase - 0.6, y: 0.35),
                endPoint: UnitPoint(x: phase + 0.6, y: 0.65)
            )

            VStack(alignment: .leading, spacing: 18) {
                userBubble(width: 168, fill: fill)
                assistantLines(trailingInsets: [24, 64, 148], fill: fill)
                userBubble(width: 122, fill: fill)
                assistantLines(trailingInsets: [40, 96], fill: fill)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Loading thread")
    }

    private func userBubble(width: CGFloat, fill: LinearGradient) -> some View {
        RoundedRectangle(cornerRadius: 19, style: .continuous)
            .fill(fill)
            .frame(width: width, height: 38)
            .frame(maxWidth: .infinity, alignment: .trailing)
    }

    private func assistantLines(trailingInsets: [CGFloat], fill: LinearGradient) -> some View {
        VStack(alignment: .leading, spacing: 9) {
            ForEach(Array(trailingInsets.enumerated()), id: \.offset) { _, inset in
                RoundedRectangle(cornerRadius: 7, style: .continuous)
                    .fill(fill)
                    .frame(height: 14)
                    .padding(.trailing, inset)
            }
        }
    }
}

/// Silent top-of-transcript boundary row for automatic older-history loading.
/// Idle it is a 1pt invisible sentinel (its `onAppear` re-arms the prefetch
/// gate when the reader reaches the very top); while a network page is
/// in-flight it shows a small unlabeled spinner. In-memory window reveals are
/// synchronous and never show it. There is no tap affordance — loading is
/// driven entirely by the scroll position.
struct GaryxEarlierHistoryLoadingIndicator: View {
    let isLoading: Bool

    var body: some View {
        Group {
            if isLoading {
                ProgressView()
                    .controlSize(.small)
                    .tint(.secondary)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 6)
                    .transition(.opacity)
            } else {
                Color.clear
                    .frame(height: 1)
            }
        }
        .accessibilityHidden(!isLoading)
        .accessibilityLabel("Loading earlier messages")
    }
}

struct GaryxEmptyConversationView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsWorkspacePicker = false

    var body: some View {
        VStack(spacing: 18) {
            Text("New Thread")
                .font(GaryxFont.title3(weight: .semibold))
                .foregroundStyle(.primary)

            workspacePicker
        }
        .frame(maxWidth: 300)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 28)
        .sheet(isPresented: $showsWorkspacePicker) {
            GaryxWorkspaceSelectSheet(
                title: "Workspace",
                path: newThreadWorkspaceBinding,
                workspacePaths: model.userWorkspacePaths,
                placeholder: "No workspace",
                allowsEmpty: true
            )
        }
        // Prefetch the catalog so the agent picker's override section is ready.
        .task(id: model.newThreadAgentTarget?.id) {
            await model.ensureNewThreadProviderModelsLoaded()
        }
        .onChange(of: model.sidebarVisible) { _, visible in
            if visible {
                showsWorkspacePicker = false
            }
        }
        .onChange(of: model.selectedThread?.id) { _, threadId in
            if threadId != nil {
                showsWorkspacePicker = false
            }
        }
        .onChange(of: model.activePanel) { _, panel in
            if panel != .chat {
                showsWorkspacePicker = false
            }
        }
    }

    private var workspacePicker: some View {
        Button {
            showsWorkspacePicker = true
        } label: {
            HStack(spacing: 10) {
                Text(model.newThreadWorkspaceLabel)
                    .font(GaryxFont.body(weight: .semibold))
                    .lineLimit(1)
                Image(systemName: "chevron.up.chevron.down")
                    .font(GaryxFont.system(size: 10, weight: .bold))
            }
            .foregroundStyle(Color(.systemBackground))
            .padding(.horizontal, 18)
            .frame(height: 46)
            .background(Color(.label), in: Capsule())
        }
        .buttonStyle(.plain)
    }

    private var newThreadWorkspaceBinding: Binding<String> {
        Binding {
            model.newThreadWorkspace
        } set: { value in
            model.setNewThreadWorkspace(value)
        }
    }

}

struct GaryxSelectedThreadEmptyConversationView: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(spacing: 14) {
            Text(model.selectedThread?.title ?? "Thread")
                .font(GaryxFont.title3(weight: .semibold))
                .foregroundStyle(.primary)
                .multilineTextAlignment(.center)
                .lineLimit(2)

            Text("No messages yet")
                .font(GaryxFont.callout())
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: 300)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 28)
    }
}

struct GaryxThinkingLabel: View {
    var text: String = "Thinking"

    var body: some View {
        GaryxShimmerText(text: text, font: GaryxFont.body())
            .frame(minHeight: 22)
    }
}

struct GaryxUserMessageLoadingBubble: View {
    private static let shimmerDuration: Double = 2.4

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: false)) { context in
            let normalized = context.date.timeIntervalSinceReferenceDate
                .truncatingRemainder(dividingBy: Self.shimmerDuration) / Self.shimmerDuration
            let phase = CGFloat(normalized) * 2.0 - 0.5
            let fill = LinearGradient(
                colors: [
                    Color.primary.opacity(0.05),
                    Color.primary.opacity(0.11),
                    Color.primary.opacity(0.05),
                ],
                startPoint: UnitPoint(x: phase - 0.6, y: 0.35),
                endPoint: UnitPoint(x: phase + 0.6, y: 0.65)
            )

            RoundedRectangle(cornerRadius: 19, style: .continuous)
                .fill(fill)
                .frame(width: 156, height: 38)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Loading message")
    }
}

/// Tail card shown when the selected thread's last run was cut off by the
/// provider's usage quota. The reset wall-clock time and countdown re-derive
/// every second from the server-provided reset time via
/// `GaryxRateLimitBannerModel`; when the gateway scheduled an auto-resend the
/// card says when it fires, otherwise a Continue button dispatches a literal
/// "continue" prompt through the regular send pipeline.
struct GaryxRateLimitBanner: View {
    let rateLimit: GaryxRenderRateLimit
    /// Dispatches the "continue" prompt. The button shows a sending state
    /// until the call returns, so a failed or no-op send re-arms the button
    /// instead of leaving it stuck; on success the run start clears the
    /// rate-limit state and removes the card.
    var onContinue: (() async -> Void)?

    @State private var sending = false
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            if let model = GaryxRateLimitBannerModel.make(from: rateLimit, now: context.date) {
                card(for: model)
            }
        }
        .onChange(of: rateLimit) { _, _ in
            // A fresh rate-limit context re-arms the Continue action.
            sending = false
        }
    }

    @ViewBuilder
    private func card(for model: GaryxRateLimitBannerModel) -> some View {
        // Compact single-row card at standard sizes. Accessibility Dynamic
        // Type is a layout input: the detail gets its line cap lifted (the
        // two-line head truncation would drop the reset hint's meaning) and
        // the Continue capsule moves below the text instead of squeezing it.
        let isAccessibilitySize = dynamicTypeSize.isAccessibilitySize

        Group {
            if isAccessibilitySize {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(alignment: .center, spacing: 10) {
                        iconChip
                        textColumn(for: model, lineLimit: nil)
                    }
                    if model.showContinue, onContinue != nil {
                        continueButton
                    }
                }
            } else {
                HStack(alignment: .center, spacing: 10) {
                    iconChip
                    textColumn(for: model, lineLimit: 2)
                    Spacer(minLength: 0)
                    if model.showContinue, onContinue != nil {
                        continueButton
                    }
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(Color(.systemBackground))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Color(.separator).opacity(0.6), lineWidth: 1)
        )
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }

    private var iconChip: some View {
        RoundedRectangle(cornerRadius: 7, style: .continuous)
            .fill(Color(.tertiarySystemFill))
            .frame(width: 26, height: 26)
            .overlay(
                Image(systemName: rateLimit.willAutoResend
                    ? "arrow.clockwise"
                    : "hourglass")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(Color(.secondaryLabel))
            )
    }

    private func textColumn(
        for model: GaryxRateLimitBannerModel,
        lineLimit: Int?
    ) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            // Compact type matching the desktop card's 13/12px scale;
            // semantic fonts so Dynamic Type still applies.
            Text(model.title)
                .font(.footnote)
                .fontWeight(.semibold)
                .foregroundStyle(Color(.label))
            Text(model.detail)
                .font(.caption)
                .monospacedDigit()
                .foregroundStyle(GaryxTheme.secondaryText)
                // Keep the card short for a verbose provider message; the
                // reset hint sits at the end, so truncate from the head.
                // Accessibility sizes pass nil and show the full text.
                .lineLimit(lineLimit)
                .truncationMode(.head)
        }
    }

    private var continueButton: some View {
        Button {
            guard !sending else { return }
            sending = true
            Task {
                await onContinue?()
                // Re-arm once the dispatch settles: a failed send leaves the
                // card mounted and the button must come back.
                sending = false
            }
        } label: {
            Text(sending ? "Sending…" : "Continue")
                .font(.caption)
                .fontWeight(.semibold)
                // The label never hyphenates; the capsule grows to fit.
                .lineLimit(1)
                .fixedSize(horizontal: true, vertical: false)
                .foregroundStyle(
                    sending ? Color(.secondaryLabel) : Color(.label)
                )
                .padding(.horizontal, 12)
                // Vertical padding instead of a fixed height so the capsule
                // grows with Dynamic Type.
                .padding(.vertical, 5)
                .background(
                    Capsule(style: .continuous)
                        .fill(Color(.systemBackground))
                )
                .overlay(
                    Capsule(style: .continuous)
                        .stroke(Color(.separator), lineWidth: 1)
                )
                // 44pt touch target extended beyond the compact visual
                // capsule so the card itself stays short.
                .contentShape(Rectangle().inset(by: -9))
        }
        .buttonStyle(.plain)
        .disabled(sending)
    }
}

struct GaryxShimmerText: View {
    let text: String
    var font: Font = GaryxFont.body()
    var baseColor: Color = GaryxTheme.secondaryText
    var peakColor: Color = Color(.label)
    var duration: Double = 2.6

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30.0, paused: false)) { context in
            let normalized = context.date.timeIntervalSinceReferenceDate
                .truncatingRemainder(dividingBy: duration) / duration
            let phase = CGFloat(normalized) * 2.0 - 0.5

            Text(text)
                .font(font)
                .foregroundStyle(
                    LinearGradient(
                        colors: [baseColor, peakColor, baseColor],
                        startPoint: UnitPoint(x: phase - 0.5, y: 0.5),
                        endPoint: UnitPoint(x: phase + 0.5, y: 0.5)
                    )
                )
        }
        .accessibilityLabel(text)
    }
}
