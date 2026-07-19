import SwiftUI
import UIKit

// Conversation task-tree sidebar: a logical-trailing push-in panel scoped to
// the open conversation. The container's public trailing pan owns arbitration;
// this view only renders the shared reveal presentation.

private enum GaryxTaskTreeSidebarMetrics {
    static let indentStep: CGFloat = 12
    /// Compact task-list glyph shared by the panel header and empty state.
    static let treeGlyph = "list.bullet"

    static func panelWidth(containerWidth: CGFloat) -> CGFloat {
        min(max(containerWidth * 0.55, 300), 420)
    }
}

extension View {
    /// Mounts the task-tree sidebar surface (shared interaction + scrim + panel)
    /// over a conversation page.
    func garyxTaskTreeSidebarSurface() -> some View {
        modifier(GaryxTaskTreeSidebarSurface())
    }
}

struct GaryxTaskTreeSidebarSurface: ViewModifier {
    @EnvironmentObject private var model: GaryxMobileModel

    func body(content: Content) -> some View {
        GaryxTaskTreeSidebarInteractionSurface(
            model: model,
            interaction: model.taskTreeRevealInteraction,
            content: content
        )
    }
}

private struct GaryxTaskTreeSidebarInteractionSurface<SurfaceContent: View>: View {
    @ObservedObject var model: GaryxMobileModel
    @ObservedObject var interaction: GaryxHorizontalRevealInteractionStore
    let content: SurfaceContent

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.garyxPrefersCrossFadeTransitions) private var prefersCrossFadeTransitions
    @Environment(\.layoutDirection) private var layoutDirection

    var body: some View {
        GeometryReader { proxy in
            let panelWidth = GaryxTaskTreeSidebarMetrics.panelWidth(containerWidth: proxy.size.width)
            let reveal = interaction.isGestureEligible
                ? interaction.reveal
                : (model.isTaskTreeSidebarOpen ? panelWidth : 0)
            let progress = panelWidth > 0 ? max(0, min(1, reveal / panelWidth)) : 0
            let leadingSign: CGFloat = layoutDirection == .leftToRight ? 1 : -1

            content
                .frame(width: proxy.size.width, height: proxy.size.height)
                // Push, not overlay: the conversation slides left in lockstep
                // with the incoming panel — the same `reveal` value drives
                // both offsets, so drag and settle animations stay in sync.
                // Reduce Motion keeps the crossfade presentation and skips
                // the push.
                .offset(x: usesCrossFade ? 0 : -leadingSign * reveal)
                .overlay {
                    if progress > 0 {
                        // While revealed, the visible conversation strip
                        // becomes one big close target and its controls are
                        // blocked. Lighter scrim than an overlay drawer: the
                        // pushed-aside content stays readable.
                        Color.black.opacity(0.18 * progress)
                            .ignoresSafeArea()
                            .contentShape(Rectangle())
                            .onTapGesture { closePanel() }
                            .accessibilityLabel("Close task tree")
                            .accessibilityAddTraits(.isButton)
                    }
                }
                .overlay(alignment: .trailing) {
                    if progress > 0 {
                        panelBody(
                            panelWidth: panelWidth,
                            reveal: reveal,
                            progress: progress,
                            safeAreaInsets: proxy.safeAreaInsets
                        )
                    }
                }
                .onAppear {
                    interaction.configure(
                        extent: panelWidth,
                        restingPosition: model.isTaskTreeSidebarOpen ? .open : .closed
                    )
                }
                .onChange(of: panelWidth) { oldWidth, newWidth in
                    guard oldWidth != newWidth else { return }
                    interaction.configure(
                        extent: newWidth,
                        restingPosition: model.isTaskTreeSidebarOpen ? .open : .closed
                    )
                }
                .onChange(of: model.isTaskTreeSidebarOpen) { _, open in
                    interaction.setTarget(
                        open ? .open : .closed,
                        animated: animatesTransitions
                    )
                }
                .onChange(of: model.selectedThread?.id) { _, _ in
                    interaction.setTarget(
                        model.isTaskTreeSidebarOpen ? .open : .closed,
                        animated: false
                    )
                }
                .task(id: model.selectedThread?.id) {
                    model.syncTaskTreeSidebarAnchor()
                    guard model.selectedThread != nil else { return }
                    await model.refreshSelectedThreadTaskForest()
                    // 5s silent refresh for the whole anchored conversation
                    // (desktop popover REFRESH_MS parity), independent of
                    // panel visibility and tree emptiness: a thread whose
                    // first task is spawned mid-conversation must re-enable
                    // the edge gesture without leaving the thread.
                    while !Task.isCancelled {
                        try? await Task.sleep(nanoseconds: 5_000_000_000)
                        guard !Task.isCancelled else { return }
                        await model.refreshSelectedThreadTaskForest()
                    }
                }
        }
    }

    private func panelBody(
        panelWidth: CGFloat,
        reveal: CGFloat,
        progress: CGFloat,
        safeAreaInsets: EdgeInsets
    ) -> some View {
        let dragActive = interaction.presentation.phase != .idle
        let leadingSign: CGFloat = layoutDirection == .leftToRight ? 1 : -1

        // Full-height rail: the material reaches the physical top and bottom
        // edges, while the header and scrolling content own their respective
        // safe-area insets. A trailing navigation rail is anchored to the
        // screen edge rather than floating like a card, so its leading edge
        // stays square at every reveal progress.
        return GaryxTaskTreeSidebarPanel(
            topSafeAreaInset: safeAreaInsets.top,
            bottomSafeAreaInset: safeAreaInsets.bottom
        )
            .frame(width: panelWidth)
            .frame(maxHeight: .infinity)
            .garyxAdaptiveGlass(
                .regular,
                isInteractive: false,
                in: Rectangle()
            )
            .clipShape(Rectangle())
            // Pre-baked gradient strip instead of `.shadow`: animated shadow
            // radii force full-screen offscreen blurs every drag frame (the
            // left drawer's frame-rate lesson).
            .overlay(alignment: .leading) {
                LinearGradient(
                    gradient: Gradient(stops: [
                        .init(color: Color.black.opacity(0.16), location: 0),
                        .init(color: Color.black.opacity(0.04), location: 0.5),
                        .init(color: Color.black.opacity(0), location: 1),
                    ]),
                    startPoint: leadingSign > 0 ? .trailing : .leading,
                    endPoint: leadingSign > 0 ? .leading : .trailing
                )
                .frame(width: 40)
                .offset(x: -leadingSign * 40)
                .opacity(Double(progress))
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            }
            // Reduce Motion: crossfade + scrim only, no interactive slide.
            .opacity(usesCrossFade ? Double(progress) : 1)
            .offset(x: usesCrossFade ? 0 : leadingSign * (panelWidth - reveal))
            .disabled(dragActive)
            .ignoresSafeArea(edges: [.top, .bottom])
            .accessibilityAddTraits(.isModal)
            .accessibilityAction(.escape) { closePanel() }
    }

    private var usesCrossFade: Bool {
        GaryxAccessibilityTransitionPolicy.usesCrossFade(
            reduceMotion: reduceMotion,
            prefersCrossFadeTransitions: prefersCrossFadeTransitions
        )
    }

    private var animatesTransitions: Bool {
        GaryxAccessibilityTransitionPolicy.animatesTransition(
            reduceMotion: reduceMotion,
            prefersCrossFadeTransitions: prefersCrossFadeTransitions
        )
    }

    private func closePanel() {
        interaction.setTarget(.closed, animated: animatesTransitions)
        model.closeTaskTreeSidebar()
    }
}

// MARK: - Panel

private struct GaryxTaskTreeSidebarPanel: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let topSafeAreaInset: CGFloat
    let bottomSafeAreaInset: CGFloat

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
                .padding(.top, topSafeAreaInset)
            Divider()
                .opacity(0.24)
            content
                .padding(.bottom, bottomSafeAreaInset)
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            Image(systemName: GaryxTaskTreeSidebarMetrics.treeGlyph)
                .font(GaryxFont.system(size: 15, weight: .semibold))
                .foregroundStyle(.secondary)
            Text("Task tree")
                .font(GaryxFont.body(weight: .semibold))
                .foregroundStyle(.primary)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .padding(.top, 10)
        .padding(.bottom, 8)
    }

    @ViewBuilder
    private var content: some View {
        let rows = model.taskTreeSidebarRows
        if rows.isEmpty {
            if model.isTaskTreeFirstLoadInFlight {
                loadingSkeleton
            } else {
                emptyState
            }
        } else {
            // The task tree scrolls without showing a scroll bar (same product
            // rule as the Mac popover).
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 2) {
                    ForEach(rows) { row in
                        GaryxTaskTreeSidebarRowView(row: row)
                    }
                }
                .padding(.horizontal, 8)
                .padding(.top, 6)
                .padding(.bottom, 16)
            }
            .scrollIndicators(.hidden)
        }
    }

    /// Skeleton shown when the panel opens before the first forest response
    /// arrives.
    private var loadingSkeleton: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(0..<4, id: \.self) { index in
                HStack(spacing: 10) {
                    Circle()
                        .fill(Color.primary.opacity(0.08))
                        .frame(width: 24, height: 24)
                    RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill(Color.primary.opacity(0.08))
                        .frame(width: index % 2 == 0 ? 170 : 130, height: 13)
                }
                .padding(.leading, index == 0 ? 0 : 12)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16)
        .padding(.top, 14)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Loading task tree")
    }

    private var emptyState: some View {
        VStack(spacing: 8) {
            Image(systemName: GaryxTaskTreeSidebarMetrics.treeGlyph)
                .font(GaryxFont.system(size: 22, weight: .medium))
                .foregroundStyle(.tertiary)
            Text("No tasks from this thread yet.")
                .font(GaryxFont.footnote())
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(.horizontal, 20)
    }
}

// MARK: - Rows

private struct GaryxTaskTreeSidebarRowView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let row: GaryxTaskTreeRow

    var body: some View {
        Button {
            Task { await model.handleTaskTreeRowTap(row) }
        } label: {
            HStack(alignment: .center, spacing: 10) {
                Color.clear
                    .frame(
                        width: GaryxTaskTreeSidebarMetrics.indentStep * CGFloat(row.indentLevel),
                        height: 1
                    )
                    .accessibilityHidden(true)

                avatar

                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        if let taskDisplayId = row.taskDisplayId {
                            Text(taskDisplayId)
                                .font(.system(size: 11, weight: .semibold, design: .monospaced))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .layoutPriority(1)
                        }
                        Text(row.title)
                            .font(GaryxFont.footnote(weight: .medium))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                        if row.isCurrent {
                            Text("Current")
                                .font(GaryxFont.system(size: 10, weight: .bold))
                                .foregroundStyle(.primary)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 1.5)
                                .background(Color.primary.opacity(0.10), in: Capsule())
                        }
                    }

                    HStack(spacing: 6) {
                        if row.kind == .sourceThread {
                            Image(systemName: "message")
                                .font(GaryxFont.system(size: 9, weight: .semibold))
                                .foregroundStyle(.secondary)
                        }
                        Text(agentLabel)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        if let status = row.status {
                            GaryxStatusPill(text: status.label, tone: status.tone)
                        }
                    }
                }

                Spacer(minLength: 0)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 7)
            .background {
                if row.isCurrent {
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .fill(Color.primary.opacity(0.055))
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(accessibilityText)
    }

    private var avatar: some View {
        let identity = model.taskTreeRowAvatar(for: row)
        return GaryxAgentAvatarView(
            agentId: identity.agentId,
            avatarDataUrl: identity.avatarDataUrl,
            label: identity.label,
            providerType: identity.providerType,
            builtIn: identity.builtIn,
            diameter: 24
        )
        .overlay(alignment: .bottomTrailing) {
            if row.isRunning {
                GaryxAvatarTypingBadge(scale: 0.64)
                    .offset(x: 2, y: 2)
            }
        }
    }

    private var agentLabel: String {
        model.taskTreeRowAvatar(for: row).label
    }

    private var accessibilityText: String {
        var parts: [String] = []
        if let taskDisplayId = row.taskDisplayId {
            parts.append(taskDisplayId)
        }
        parts.append(row.title)
        if row.kind == .sourceThread {
            parts.append(agentLabel)
        }
        if let status = row.status {
            parts.append(status.label)
        }
        if row.isCurrent {
            parts.append("current")
        }
        return parts.joined(separator: ", ")
    }
}

// MARK: - Status presentation

/// Status pill text/tone for task-tree rows (previously shared with the
/// removed Tasks management panel).
extension GaryxTaskStatus {
    var label: String {
        switch self {
        case .todo:
            "Todo"
        case .inProgress:
            "In Progress"
        case .inReview:
            "In Review"
        case .done:
            "Done"
        }
    }

    var tone: GaryxStatusPill.Tone {
        switch self {
        case .todo:
            .muted
        case .inProgress:
            .warning
        case .inReview:
            .danger
        case .done:
            .good
        }
    }
}
