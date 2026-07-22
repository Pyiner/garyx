import Foundation
import SwiftUI

/// Transcript loading placeholder: a chat-shaped skeleton (user pill on the
/// trailing edge, assistant text lines on the leading edge) swept by the same
/// soft shimmer treatment as `GaryxShimmerText`, instead of a bare spinner.
struct GaryxThreadHistoryLoadingView: View {
    @Environment(\.garyxMotion) private var motion

    var body: some View {
        TimelineView(
            .animation(
                minimumInterval: GaryxMotion.timelineMinimumInterval,
                paused: motion.pausesContinuousMotion(.loadingShimmer)
            )
        ) { context in
            let shimmerDuration = motion.cycleDuration(.loadingShimmer)
            let normalized = context.date.timeIntervalSinceReferenceDate
                .truncatingRemainder(dividingBy: shimmerDuration) / shimmerDuration
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
        .garyxSheet(isPresented: $showsWorkspacePicker) {
            GaryxWorkspaceSelectSheet(
                title: "Workspace",
                path: draftWorkspaceBinding,
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
                    .garyxReadingLineLimit()
                Image(systemName: "chevron.up.chevron.down")
                    .font(GaryxFont.fixedSystem(size: 10, weight: .bold))
            }
            .foregroundStyle(Color(.systemBackground))
            .padding(.horizontal, 18)
            .padding(.vertical, 12)
            .frame(minHeight: 46)
            .background(Color(.label), in: Capsule())
        }
        .buttonStyle(GaryxPressableRowStyle())
    }

    /// The picker writes "" only from the explicit "No workspace" row, so an
    /// empty set maps to the explicit `none` tri-state, never to unresolved.
    private var draftWorkspaceBinding: Binding<String> {
        Binding {
            model.newThreadWorkspaceSelection.workspacePath ?? ""
        } set: { value in
            if value.isEmpty {
                model.selectDraftNoWorkspace()
            } else {
                model.selectDraftWorkspace(value)
            }
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
                .garyxReadingLineLimit(2)

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
    @Environment(\.garyxMotion) private var motion

    var body: some View {
        TimelineView(
            .animation(
                minimumInterval: GaryxMotion.timelineMinimumInterval,
                paused: motion.pausesContinuousMotion(.loadingShimmer)
            )
        ) { context in
            let shimmerDuration = motion.cycleDuration(.loadingShimmer)
            let normalized = context.date.timeIntervalSinceReferenceDate
                .truncatingRemainder(dividingBy: shimmerDuration) / shimmerDuration
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
/// card says when it fires. Claude Code also exposes the provider-owned
/// account selector so a healthy account can resume all quota-paused threads.
struct GaryxRateLimitBanner: View {
    let rateLimit: GaryxRenderRateLimit
    /// Makes the same durable SQL recovery generation due immediately. The
    /// button never creates an independent ordinary user message.
    var onContinue: (() async throws -> GaryxQuotaRecoveryRetryResult)?

    @EnvironmentObject private var mobileModel: GaryxMobileModel
    @State private var sending = false
    @State private var showsAccountSwitcher = false
    @State private var recoveryNotice: String?
    @State private var recoveryFeedback: String?
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            if let model = GaryxRateLimitBannerModel.make(from: rateLimit, now: context.date) {
                card(for: model)
            }
        }
        .onChange(of: recoveryContextID) { _, _ in
            // A fresh rate-limit context re-arms the Continue action.
            sending = false
            recoveryNotice = nil
            recoveryFeedback = nil
        }
        .garyxSheet(isPresented: $showsAccountSwitcher) {
            GaryxClaudeCodeAccountsSheet(selectionOnly: true) { result in
                if !result.selectionChanged {
                    recoveryNotice = nil
                } else if result.recoveryWarning != nil {
                    recoveryNotice = "Account switched. Retry paused threads manually."
                } else if result.recovery.matchedThreads > 0 {
                    recoveryNotice = "Resuming \(result.recovery.matchedThreads) paused threads…"
                } else {
                    recoveryNotice = "Account switched."
                }
            }
        }
    }

    @ViewBuilder
    private func card(for model: GaryxRateLimitBannerModel) -> some View {
        let isAccessibilitySize = dynamicTypeSize.isAccessibilitySize

        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .center, spacing: 12) {
                GaryxProviderAgentAvatarView(
                    providerType: rateLimit.provider ?? "",
                    diameter: 36
                )

                Text(model.title)
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                    .fixedSize(horizontal: false, vertical: true)

                Spacer(minLength: 8)

                if !isAccessibilitySize, model.showContinue, onContinue != nil {
                    continueButton
                }
            }

            Text(model.detail)
                .font(GaryxFont.footnote())
                .monospacedDigit()
                .foregroundStyle(GaryxTheme.secondaryText)
                .fixedSize(horizontal: false, vertical: true)

            if let recoveryFeedback {
                Text(recoveryFeedback)
                    .font(GaryxFont.caption())
                    .foregroundStyle(GaryxTheme.secondaryText)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if isAccessibilitySize, model.showContinue, onContinue != nil {
                continueButton
            }

            if isClaudeCode {
                Divider()
                    .overlay(Color(.separator).opacity(0.55))

                Button {
                    recoveryNotice = nil
                    showsAccountSwitcher = true
                    Task { await mobileModel.loadClaudeCodeAccounts() }
                } label: {
                    HStack(spacing: 12) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Claude Code account")
                                .font(GaryxFont.caption())
                                .foregroundStyle(.secondary)
                            Text(selectedAccountName)
                                .font(GaryxFont.subheadline(weight: .semibold))
                                .foregroundStyle(.primary)
                                .fixedSize(horizontal: false, vertical: true)
                        }

                        Spacer(minLength: 8)

                        Text("Switch account")
                            .font(GaryxFont.caption(weight: .semibold))
                            .foregroundStyle(.primary)
                            .garyxReadingLineLimit()
                        Image(systemName: "chevron.right")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                    }
                    .frame(minHeight: 44)
                    .contentShape(Rectangle())
                }
                .buttonStyle(GaryxPressableRowStyle())
                .disabled(mobileModel.isMutatingClaudeCodeAccount)

                Text(
                    recoveryNotice
                        ?? "Switching accounts resumes every Claude thread paused by quota."
                )
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(16)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(Color(.systemBackground))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(Color(.separator).opacity(0.6), lineWidth: 1)
        )
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var continueButton: some View {
        Button {
            guard !sending else { return }
            sending = true
            recoveryFeedback = nil
            Task {
                do {
                    if let onContinue {
                        switch try await onContinue() {
                        case .settled:
                            recoveryFeedback = "This recovery already finished. Send a new message to continue."
                        case .unsupported:
                            recoveryFeedback = "Update the Garyx gateway to resume from this quota card."
                        case .accepted:
                            break
                        }
                    }
                } catch {
                    recoveryFeedback = "Couldn't resume this thread. Try again."
                }
                // Re-arm once the dispatch settles: a terminal or failed send
                // leaves the card mounted and the button must come back.
                sending = false
            }
        } label: {
            Text(sending ? "Sending…" : "Continue")
                .font(.caption)
                .fontWeight(.semibold)
                // The label never hyphenates; the capsule grows to fit.
                .garyxReadingLineLimit()
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
        .buttonStyle(GaryxPressableRowStyle(prepares: .messageSendCommitted))
        .disabled(sending)
    }

    private var isClaudeCode: Bool {
        providerPresentation.kind == .claude
    }

    private var providerPresentation: GaryxProviderPresentation {
        GaryxProviderPresentation.make(providerType: rateLimit.provider ?? "")
    }

    private var recoveryContextID: String {
        [
            rateLimit.provider ?? "",
            rateLimit.window ?? "",
            rateLimit.resetAt ?? "",
            rateLimit.recoveryGeneration ?? ""
        ].joined(separator: "|")
    }

    private var selectedAccountName: String {
        if let account = mobileModel.claudeCodeAccounts?.selectedAccount {
            return account.name
        }
        return mobileModel.isLoadingClaudeCodeAccounts ? "Loading account…" : "Choose account"
    }
}

struct GaryxShimmerText: View {
    @Environment(\.garyxMotion) private var motion
    let text: String
    var font: Font = GaryxFont.body()
    var baseColor: Color = GaryxTheme.secondaryText
    var peakColor: Color = Color(.label)

    var body: some View {
        TimelineView(
            .animation(
                minimumInterval: GaryxMotion.timelineMinimumInterval,
                paused: motion.pausesContinuousMotion(.thinkingShimmer)
            )
        ) { context in
            let duration = motion.cycleDuration(.thinkingShimmer)
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
