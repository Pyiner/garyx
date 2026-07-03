import SwiftUI
import UIKit

// Conversation task-tree sidebar: a trailing push-in panel scoped to the open
// conversation — the panel slides in from the right edge and pushes the
// conversation content left in lockstep (not an overlay). The right-edge
// swipe mirrors the left navigation drawer's proven gesture parameters (edge
// zone 24pt, min distance 18pt, axis decision 14pt at 1.5x dominance, open
// 22%/35%, close 12%/28%, @GestureState cancel self-heal) so the two edges
// feel symmetric.

private enum GaryxTaskTreeDragAxis {
    case horizontal
    case vertical
}

private enum GaryxTaskTreeSidebarMetrics {
    static let edgeGestureWidth: CGFloat = 24
    static let axisDecisionDistance: CGFloat = 14
    static let axisDecisionRatio: CGFloat = 1.5
    static let leadingCornerRadius: CGFloat = 28
    static let indentStep: CGFloat = 12

    static func panelWidth(containerWidth: CGFloat) -> CGFloat {
        min(max(containerWidth * 0.55, 300), 420)
    }
}

/// Clip whose bounds are outset through the surrounding safe areas so the
/// panel's glass reaches the physical top/bottom/trailing edges while content
/// keeps safe-area layout; only the leading corners are rounded (the panel
/// hangs off the trailing edge).
private struct GaryxTaskTreePanelClipShape: Shape {
    var leadingCornerRadius: CGFloat
    var safeAreaOutsets: EdgeInsets

    func path(in rect: CGRect) -> Path {
        let expanded = CGRect(
            x: rect.minX - safeAreaOutsets.leading,
            y: rect.minY - safeAreaOutsets.top,
            width: rect.width + safeAreaOutsets.leading + safeAreaOutsets.trailing,
            height: rect.height + safeAreaOutsets.top + safeAreaOutsets.bottom
        )
        return UnevenRoundedRectangle(
            topLeadingRadius: leadingCornerRadius,
            bottomLeadingRadius: leadingCornerRadius,
            bottomTrailingRadius: 0,
            topTrailingRadius: 0,
            style: .continuous
        )
        .path(in: expanded)
    }
}

extension View {
    /// Mounts the task-tree sidebar surface (opening gesture + scrim + panel)
    /// over a conversation page.
    func garyxTaskTreeSidebarSurface() -> some View {
        modifier(GaryxTaskTreeSidebarSurface())
    }
}

struct GaryxTaskTreeSidebarSurface: ViewModifier {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.garyxSidebarDragActive) private var drawerDragActive
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    @State private var dragOffset: CGFloat = 0
    @State private var dragAxis: GaryxTaskTreeDragAxis?
    @State private var containerGlobalFrame: CGRect = .zero
    /// Auto-resetting drag liveness: `DragGesture.onEnded` is skipped when the
    /// system cancels a gesture, which would leave `dragAxis` stuck and the
    /// panel half-revealed; `@GestureState` always resets, so the
    /// `onChange(of: dragLive)` below cleans up after cancellation (the left
    /// drawer's documented fix, reused).
    @GestureState private var dragLive = false

    func body(content: Content) -> some View {
        GeometryReader { proxy in
            let panelWidth = GaryxTaskTreeSidebarMetrics.panelWidth(containerWidth: proxy.size.width)
            let reveal = revealWidth(panelWidth: panelWidth)
            let progress = panelWidth > 0 ? max(0, min(1, reveal / panelWidth)) : 0

            content
                .frame(width: proxy.size.width, height: proxy.size.height)
                .simultaneousGesture(openingGesture(panelWidth: panelWidth))
                // Push, not overlay: the conversation slides left in lockstep
                // with the incoming panel — the same `reveal` value drives
                // both offsets, so drag and settle animations stay in sync.
                // Reduce Motion keeps the crossfade presentation and skips
                // the push.
                .offset(x: reduceMotion ? 0 : -reveal)
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
                            .simultaneousGesture(closingGesture(panelWidth: panelWidth))
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
                .onChange(of: dragLive) { _, live in
                    guard !live, dragAxis != nil else { return }
                    dragAxis = nil
                    resetDrag()
                }
                .onChange(of: model.selectedThread?.id) { _, _ in
                    dragAxis = nil
                    dragOffset = 0
                }
                .onGeometryChange(for: CGRect.self) { geometry in
                    geometry.frame(in: .global)
                } action: { frame in
                    containerGlobalFrame = frame
                }
                .task(id: model.selectedThread?.id) {
                    model.syncTaskTreeSidebarAnchor()
                    await model.refreshSelectedThreadTaskForest()
                }
                .task(id: "\(model.isTaskTreeSidebarOpen)|\(model.selectedThread?.id ?? "")") {
                    // 5s silent refresh while the panel is open (desktop
                    // REFRESH_MS parity); a known-empty tree suspends the loop
                    // until the thread changes or a local task mutation calls
                    // noteTaskTreeLocalMutation.
                    guard model.isTaskTreeSidebarOpen else { return }
                    while !Task.isCancelled, model.isTaskTreeSidebarOpen {
                        try? await Task.sleep(nanoseconds: 5_000_000_000)
                        guard !Task.isCancelled, model.isTaskTreeSidebarOpen else { return }
                        guard model.shouldContinueTaskTreePolling else { continue }
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
        let clipOutsets = EdgeInsets(
            top: safeAreaInsets.top,
            leading: 0,
            bottom: safeAreaInsets.bottom,
            trailing: safeAreaInsets.trailing
        )
        let dragActive = dragAxis == .horizontal

        return GaryxTaskTreeSidebarPanel(onClose: { closePanel() })
            .frame(width: panelWidth)
            .frame(maxHeight: .infinity)
            .garyxAdaptiveGlass(
                .regular,
                isInteractive: false,
                fallbackMaterial: .ultraThinMaterial,
                in: Rectangle()
            )
            .clipShape(
                GaryxTaskTreePanelClipShape(
                    leadingCornerRadius: GaryxTaskTreeSidebarMetrics.leadingCornerRadius * progress,
                    safeAreaOutsets: clipOutsets
                )
            )
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
                    startPoint: .trailing,
                    endPoint: .leading
                )
                .frame(width: 40)
                .padding(.vertical, -safeAreaInsets.top - safeAreaInsets.bottom)
                .offset(x: -40)
                .opacity(Double(progress))
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            }
            // Reduce Motion: crossfade + scrim only, no interactive slide.
            .opacity(reduceMotion ? Double(progress) : 1)
            .offset(x: reduceMotion ? 0 : panelWidth - reveal)
            .disabled(dragActive)
            .simultaneousGesture(closingGesture(panelWidth: panelWidth))
            .accessibilityAddTraits(.isModal)
            .accessibilityAction(.escape) { closePanel() }
    }

    /// Revealed panel width for the current open/drag state.
    private func revealWidth(panelWidth: CGFloat) -> CGFloat {
        if model.isTaskTreeSidebarOpen {
            return max(0, min(panelWidth, panelWidth - max(0, dragOffset)))
        }
        return max(0, min(panelWidth, -min(0, dragOffset)))
    }

    private func openingGesture(panelWidth: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 18, coordinateSpace: .global)
            .updating($dragLive) { _, state, _ in
                state = true
            }
            .onChanged { value in
                guard !model.isTaskTreeSidebarOpen, canStartOpeningDrag else { return }
                if dragAxis == nil {
                    dragAxis = decideAxis(
                        translation: value.translation,
                        startLocation: value.startLocation,
                        opening: true
                    )
                }
                guard dragAxis == .horizontal else { return }
                dragOffset = min(0, max(-panelWidth, value.translation.width))
            }
            .onEnded { value in
                guard !model.isTaskTreeSidebarOpen else { return }
                defer { dragAxis = nil }
                guard dragAxis == .horizontal else {
                    resetDrag()
                    return
                }
                let shouldOpen = -value.translation.width > panelWidth * 0.22
                    || -value.predictedEndTranslation.width > panelWidth * 0.35
                finishGesture(open: shouldOpen)
            }
    }

    private func closingGesture(panelWidth: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 18, coordinateSpace: .global)
            .updating($dragLive) { _, state, _ in
                state = true
            }
            .onChanged { value in
                guard model.isTaskTreeSidebarOpen else { return }
                if dragAxis == nil {
                    dragAxis = decideAxis(
                        translation: value.translation,
                        startLocation: value.startLocation,
                        opening: false
                    )
                }
                guard dragAxis == .horizontal else { return }
                dragOffset = max(0, min(panelWidth, value.translation.width))
            }
            .onEnded { value in
                guard model.isTaskTreeSidebarOpen else { return }
                defer { dragAxis = nil }
                guard dragAxis == .horizontal else {
                    resetDrag()
                    return
                }
                let shouldClose = value.translation.width > panelWidth * 0.12
                    || value.predictedEndTranslation.width > panelWidth * 0.28
                finishGesture(open: !shouldClose)
            }
    }

    /// The gesture qualifies only when the tree can open: not while the left
    /// drawer is dragging, and never on a known-empty tree (an unknown tree
    /// opens onto the loading state while the first fetch is in flight).
    private var canStartOpeningDrag: Bool {
        guard !drawerDragActive else { return false }
        return model.taskTreeForestPage == nil || model.isTaskTreeSidebarAvailable
    }

    private func decideAxis(
        translation: CGSize,
        startLocation: CGPoint,
        opening: Bool
    ) -> GaryxTaskTreeDragAxis? {
        let horizontal = translation.width
        let vertical = translation.height
        let horizontalMag = abs(horizontal)
        let verticalMag = abs(vertical)
        let dominant = max(horizontalMag, verticalMag)
        guard dominant >= GaryxTaskTreeSidebarMetrics.axisDecisionDistance else { return nil }
        // Opening competes with vertical transcript scrolling and stays
        // strict; closing an open panel is an unambiguous intent.
        let ratio = opening ? GaryxTaskTreeSidebarMetrics.axisDecisionRatio : 1.0
        guard horizontalMag > verticalMag * ratio else {
            return .vertical
        }
        if opening {
            guard horizontal < 0,
                  startLocation.x >= containerGlobalFrame.maxX
                      - GaryxTaskTreeSidebarMetrics.edgeGestureWidth else {
                return .vertical
            }
        } else {
            guard horizontal > 0 else { return .vertical }
        }
        return .horizontal
    }

    private func finishGesture(open: Bool) {
        withAnimation(reduceMotion ? .easeInOut(duration: 0.2) : GaryxMobileMotion.sidebar) {
            if open {
                model.openTaskTreeSidebar()
            } else {
                model.closeTaskTreeSidebar()
            }
            dragOffset = 0
        }
    }

    private func resetDrag() {
        withAnimation(GaryxMobileMotion.sidebar) {
            dragOffset = 0
        }
    }

    private func closePanel() {
        finishGesture(open: false)
    }
}

// MARK: - Panel

private struct GaryxTaskTreeSidebarPanel: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let onClose: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider()
                .opacity(0.35)
            content
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            Image(systemName: "list.bullet.indent")
                .font(GaryxFont.system(size: 14, weight: .semibold))
                .foregroundStyle(.secondary)
            Text("Task tree")
                .font(GaryxFont.subheadline(weight: .semibold))
                .foregroundStyle(.primary)
            if model.taskTreeActiveBadgeCount > 0 {
                Text("\(model.taskTreeActiveBadgeCount)")
                    .font(GaryxFont.system(size: 11, weight: .bold))
                    .foregroundStyle(GaryxTheme.accent)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(GaryxTheme.accent.opacity(0.12), in: Capsule())
                    .accessibilityLabel("\(model.taskTreeActiveBadgeCount) active tasks")
            }
            Spacer(minLength: 0)
            Button(action: onClose) {
                Image(systemName: "xmark")
                    .font(GaryxFont.system(size: 12, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 30, height: 30)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Close task tree")
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
            Image(systemName: "checklist")
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
                                .foregroundStyle(.white)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 1.5)
                                .background(GaryxTheme.accent, in: Capsule())
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
                        .fill(GaryxTheme.accent.opacity(0.10))
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
            kind: identity.kind,
            label: identity.label,
            providerType: identity.providerType,
            builtIn: identity.builtIn,
            diameter: 24
        )
        .overlay(alignment: .bottomTrailing) {
            if row.isRunning {
                GaryxAvatarTypingBadge()
                    .scaleEffect(0.8)
                    .offset(x: 3, y: 3)
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

// MARK: - Header entry point

/// Tree-shaped header button shown while the current thread's task tree is
/// non-empty; carries the active-count badge and makes the invisible edge
/// gesture discoverable.
struct GaryxTaskTreeHeaderButton: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        if model.isTaskTreeSidebarAvailable {
            Button {
                model.toggleTaskTreeSidebar()
            } label: {
                GaryxToolbarIcon(systemName: "list.bullet.indent")
                    .overlay(alignment: .topTrailing) {
                        if model.taskTreeActiveBadgeCount > 0 {
                            Text("\(min(model.taskTreeActiveBadgeCount, 99))")
                                .font(GaryxFont.system(size: 10, weight: .bold))
                                .foregroundStyle(.white)
                                .padding(.horizontal, 5)
                                .padding(.vertical, 1.5)
                                .background(GaryxTheme.accent, in: Capsule())
                                .offset(x: 4, y: -2)
                        }
                    }
            }
            .buttonStyle(.plain)
            .accessibilityLabel(
                model.taskTreeActiveBadgeCount > 0
                    ? "Task tree, \(model.taskTreeActiveBadgeCount) active tasks"
                    : "Task tree"
            )
        }
    }
}
